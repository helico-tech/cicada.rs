//! A generated scenario, the one script actor, and the runner: several real nodes on one thread and paused tokio time,
//! each reaching one shared `InMemoryStore` through its own [`FaultStore`].
//!
//! A message's payload is its script, so new behaviour needs a new action and not a new actor. Message ids come from the
//! position of a send (`m3`, `m3.0`, `m3.0.1`), so no random id enters a run and a repeated run gives an equal log.

use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::NodeConfig;
use cicada::store::{CreateResult, Message, NodeId, OutcomeRecord, Page, PageCursor, Store};
use cicada_test::{InMemoryStore, TokioClock, default_start};
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use crate::faults::{Event, Fault, FaultStore, HandlerEnd, Log, Method, Rule};

/// The instances of a scenario, all of the one type.
pub const KEYS: [&str; 3] = ["a", "b", "c"];

/// The lease of every node.
pub const LEASE_TTL: Duration = Duration::from_secs(3);
/// The longest turn; a longer one ends in a timeout.
pub const MAX_TURN: Duration = Duration::from_secs(10);
/// The longest delay of a send.
pub const MAX_SEND_DELAY: Duration = Duration::from_secs(5);
/// The wait before a retry.
pub const BACKOFF: Duration = Duration::from_secs(1);
/// The most handler runs a retry policy asks for.
pub const MAX_ATTEMPTS: u32 = 3;
/// The longest injected delay. It is longer than the lease, so that a commit can arrive after the lease expired.
pub const MAX_FAULT_DELAY: Duration = Duration::from_secs(8);

/// How long outcomes are kept: far longer than a run, so that none disappears before the check.
const RETENTION: Duration = Duration::from_secs(365 * 24 * 3600);
/// How often the drain looks whether the run has ended.
const DRAIN_STEP: Duration = Duration::from_secs(1);
const PAGE: usize = Page::<Message>::DEFAULT_SIZE;

/// One step of a script. `to` is an index into [`KEYS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum Action {
    /// Sends a message that runs `actions`.
    Send {
        /// The target.
        to: usize,
        /// The delay of the send.
        delay_ms: u64,
        /// The handler runs its retry policy asks for; `None` is no policy.
        retry: Option<u32>,
        /// The script of the message.
        actions: Vec<Action>,
    },
    /// Creates an instance.
    Create {
        /// The instance.
        to: usize,
    },
    /// Stops this instance when the turn commits.
    Stop,
    /// Waits on virtual time.
    Sleep {
        /// How long.
        ms: u64,
    },
    /// Fails the handler while the message has failed fewer than `times` times.
    Fail {
        /// How many runs fail.
        times: u32,
    },
    /// Panics while the message has failed fewer than `times` times.
    Panic {
        /// How many runs panic.
        times: u32,
    },
}

/// One step of a scenario. `node` is taken modulo the number of nodes, `to` is an index into [`KEYS`], and `back`
/// counts back through the client's earlier messages.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Lets virtual time pass.
    Advance(Duration),
    /// A client sends a new message.
    Send {
        /// The node it goes through.
        node: usize,
        /// The target.
        to: usize,
        /// The delay of the send.
        delay_ms: u64,
        /// The handler runs its retry policy asks for.
        retry: Option<u32>,
        /// The script of the message.
        actions: Vec<Action>,
    },
    /// A client sends an earlier message again, as after a lost acknowledgement.
    Resend {
        /// The node it goes through.
        node: usize,
        /// Which message.
        back: usize,
    },
    /// Creates an instance.
    Create {
        /// The node it goes through.
        node: usize,
        /// The instance.
        to: usize,
    },
    /// Deletes an instance.
    Delete {
        /// The node it goes through.
        node: usize,
        /// The instance.
        to: usize,
    },
    /// Cancels a message.
    Cancel {
        /// The node it goes through.
        node: usize,
        /// Which message.
        back: usize,
    },
    /// Makes a message due now.
    DeliverNow {
        /// The node it goes through.
        node: usize,
        /// Which message.
        back: usize,
    },
    /// Pauses or resumes a mailbox.
    SetPaused {
        /// The node it goes through.
        node: usize,
        /// The instance.
        to: usize,
        /// Paused or resumed.
        paused: bool,
    },
    /// Purges a mailbox.
    Purge {
        /// The node it goes through.
        node: usize,
        /// The instance.
        to: usize,
    },
    /// Asks to cancel the running turn.
    RequestCancel {
        /// The node it goes through.
        node: usize,
        /// The instance.
        to: usize,
    },
}

/// A fault of a scenario. `node` is taken modulo the number of nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedFault {
    /// The node.
    pub node: usize,
    /// The rule.
    pub rule: Rule,
}

/// A whole run, as a value that proptest can shrink.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    /// How many nodes, at least one.
    pub nodes: usize,
    /// What clients do and how time passes.
    pub steps: Vec<Step>,
    /// What goes wrong. A `Dead` rule on node 0 is ignored, as one node must stay alive.
    pub faults: Vec<PlannedFault>,
}

// ---- the actor ----

/// type `script`, key `{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    /// The key.
    pub id: String,
}

impl ActorRef for Script {
    const TYPE: &'static str = "script";
    type Message = ScriptMsg;
}

fn script(to: usize) -> Script {
    Script { id: KEYS[to % KEYS.len()].to_owned() }
}

/// The one message type.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ScriptMsg {
    /// Runs the actions, in order.
    Run {
        /// The script.
        actions: Vec<Action>,
    },
}

/// The ids of the messages whose turn committed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScriptState {
    /// The ids.
    #[serde(default)]
    pub ids: Vec<String>,
}

/// Runs scripts on one node.
pub struct ScriptActor {
    node: String,
    log: Log,
}

/// Records the end of a handler, also when it is dropped or panics.
struct HandlerRun {
    log: Log,
    node: String,
    address: String,
    id: String,
    attempt: u32,
    how: Option<HandlerEnd>,
}

impl Drop for HandlerRun {
    fn drop(&mut self) {
        let dropped = if std::thread::panicking() { HandlerEnd::Panicked } else { HandlerEnd::Dropped };
        self.log.push(Event::HandlerEnd {
            node: self.node.clone(),
            address: self.address.clone(),
            id: self.id.clone(),
            attempt: self.attempt,
            how: self.how.unwrap_or(dropped),
        });
    }
}

fn options(id: String, delay_ms: u64, retry: Option<u32>) -> SendOptions {
    let mut options = SendOptions::new().id(MessageId::new(id).expect("an id is not blank"));
    if delay_ms > 0 {
        options = options.delay(Duration::from_millis(delay_ms));
    }
    if let Some(max_attempts) = retry {
        let backoff = Backoff::fixed(BACKOFF).expect("a valid backoff");
        options = options.retry(RetryPolicy::new(max_attempts, backoff).expect("a valid policy"));
    }
    options
}

impl ScriptActor {
    async fn perform(&self, turn: &mut Turn<Self>, actions: Vec<Action>) -> HandlerResult {
        let message = turn.message().clone();
        for (index, action) in actions.into_iter().enumerate() {
            match action {
                Action::Send { to, delay_ms, retry, actions } => {
                    let child = options(format!("{}.{index}", message.id), delay_ms, retry);
                    turn.send_with(&script(to), ScriptMsg::Run { actions }, child)?;
                }
                Action::Create { to } => turn.create(&script(to), &ScriptState::default())?,
                Action::Stop => turn.stop(),
                Action::Sleep { ms } => sleep(Duration::from_millis(ms)).await,
                Action::Fail { times } if message.failures < times => {
                    return Err(HandlerError::msg("scripted failure"));
                }
                Action::Panic { times } if message.failures < times => panic!("scripted panic"),
                Action::Fail { .. } | Action::Panic { .. } => {}
            }
        }
        Ok(())
    }
}

impl Actor for ScriptActor {
    type Ref = Script;
    type State = ScriptState;
    type Message = ScriptMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;
    const MAX_TURN: Option<Duration> = Some(MAX_TURN);

    async fn handle(&self, turn: &mut Turn<Self>, message: ScriptMsg) -> HandlerResult {
        let ScriptMsg::Run { actions } = message;
        let id = turn.message().id.to_string();
        let address = turn.self_address().to_string();
        let attempt = turn.message().attempt;
        self.log.push(Event::HandlerStart {
            node: self.node.clone(),
            address: address.clone(),
            id: id.clone(),
            attempt,
        });
        let mut run =
            HandlerRun { log: self.log.clone(), node: self.node.clone(), address, id: id.clone(), attempt, how: None };
        turn.state_mut().ids.push(id);
        let result = self.perform(turn, actions).await;
        run.how = Some(if result.is_ok() { HandlerEnd::Returned } else { HandlerEnd::Failed });
        result
    }
}

// ---- the runner ----

struct Node {
    store: Arc<FaultStore>,
    system: ActorSystem,
}

struct Runner {
    memory: Arc<InMemoryStore>,
    nodes: Vec<Node>,
    log: Log,
    /// What clients sent: the message and its target.
    sent: Vec<(MessageId, Script, ScriptMsg, SendOptions)>,
}

fn empty_state() -> Option<Value> {
    Some(json!({ "ids": [] }))
}

impl Runner {
    async fn start(scenario: &Scenario) -> Self {
        let log = Log::default();
        let clock = Arc::new(TokioClock::new(default_start()));
        let memory = Arc::new(InMemoryStore::new(clock, RETENTION));
        let count = scenario.nodes.max(1);
        let mut nodes = Vec::new();
        for index in 0..count {
            let name = format!("n{index}");
            let rules = scenario
                .faults
                .iter()
                .filter(|f| f.node % count == index && (index != 0 || f.rule.fault != Fault::Dead))
                .map(|f| f.rule)
                .collect();
            let store = FaultStore::new(memory.clone(), name.clone(), rules, log.clone());
            let config = NodeConfig {
                node_id: NodeId::new(name.clone()).expect("a name is not blank"),
                lease_ttl: LEASE_TTL,
                poll_interval: Duration::from_millis(200),
                ..NodeConfig::default()
            };
            let actor = ScriptActor { node: name, log: log.clone() };
            let system = ActorSystem::builder(store.clone()).config(config).host(ActorType::new(actor)).build();
            system.start().await.expect("the node starts");
            nodes.push(Node { store, system });
        }
        for to in 0..KEYS.len() {
            let address = script(to).address().expect("a valid address");
            let created = memory.create(&address, empty_state()).await.expect("the store is up");
            assert_eq!(created, CreateResult::Created);
        }
        for node in &nodes {
            node.store.arm();
        }
        Self { memory, nodes, log, sent: Vec::new() }
    }

    fn node(&self, index: usize) -> &Node {
        &self.nodes[index % self.nodes.len()]
    }

    fn earlier(&self, back: usize) -> Option<&(MessageId, Script, ScriptMsg, SendOptions)> {
        self.sent.len().checked_sub(1 + back % self.sent.len().max(1)).map(|index| &self.sent[index])
    }

    /// Runs a step. A client gets whatever the store answers and moves on, as the checks read the log.
    async fn step(&mut self, step: &Step) {
        match step {
            Step::Advance(by) => sleep(*by).await,
            Step::Send { node, to, delay_ms, retry, actions } => {
                let id = format!("m{}", self.sent.len());
                let to = script(*to);
                let message = ScriptMsg::Run { actions: actions.clone() };
                let send = options(id.clone(), *delay_ms, *retry);
                let _ = self.node(*node).system.send_with(&to, message.clone(), send.clone()).await;
                self.sent.push((MessageId::new(id).expect("an id is not blank"), to, message, send));
            }
            Step::Resend { node, back } => {
                if let Some((_, to, message, send)) = self.earlier(*back) {
                    let _ = self.node(*node).system.send_with(to, message.clone(), send.clone()).await;
                }
            }
            Step::Create { node, to } => {
                let address = script(*to).address().expect("a valid address");
                let _ = self.node(*node).store.create(&address, empty_state()).await;
            }
            Step::Delete { node, to } => {
                let address = script(*to).address().expect("a valid address");
                let _ = self.node(*node).store.delete(&address).await;
            }
            Step::Cancel { node, back } => {
                if let Some((id, ..)) = self.earlier(*back) {
                    let _ = self.node(*node).store.cancel(id).await;
                }
            }
            Step::DeliverNow { node, back } => {
                if let Some((id, ..)) = self.earlier(*back) {
                    let _ = self.node(*node).store.deliver_now(id).await;
                }
            }
            Step::SetPaused { node, to, paused } => {
                let address = script(*to).address().expect("a valid address");
                let _ = self.node(*node).store.set_paused(&address, *paused).await;
            }
            Step::Purge { node, to } => {
                let address = script(*to).address().expect("a valid address");
                let _ = self.node(*node).store.purge(&address).await;
            }
            Step::RequestCancel { node, to } => {
                let address = script(*to).address().expect("a valid address");
                let _ = self.node(*node).store.request_cancel(&address).await;
            }
        }
        tokio::task::yield_now().await;
    }

    async fn all_pending(&self) -> Vec<Message> {
        let mut all = Vec::new();
        let mut cursor: Option<PageCursor> = None;
        loop {
            let page = self.memory.list_pending("", cursor.as_ref(), PAGE).await.expect("the store is up");
            all.extend(page.items);
            cursor = page.next;
            if cursor.is_none() {
                return all;
            }
        }
    }

    async fn all_outcomes(&self) -> Vec<OutcomeRecord> {
        let mut all = Vec::new();
        let mut cursor: Option<PageCursor> = None;
        loop {
            let page =
                self.memory.outcomes(None, "", default_start(), cursor.as_ref(), PAGE).await.expect("the store is up");
            all.extend(page.items);
            cursor = page.next;
            if cursor.is_none() {
                return all;
            }
        }
    }

    /// Lifts the faults except `Dead`, resumes every mailbox and lets time pass until the run has ended, or the bound.
    async fn drain(&self, bound: Duration) {
        for node in &self.nodes {
            node.store.heal();
        }
        for to in 0..KEYS.len() {
            let address = script(to).address().expect("a valid address");
            self.memory.set_paused(&address, false).await.expect("the store is up");
        }
        let deadline = Instant::now() + bound;
        while Instant::now() < deadline {
            sleep(DRAIN_STEP).await;
            if self.log.open_handlers() == 0 && self.all_pending().await.is_empty() {
                break;
            }
        }
    }

    /// Writes what the store holds now to the log.
    async fn record_final_state(&self) {
        let now = self.memory.now().await.expect("the store is up");
        for outcome in self.all_outcomes().await {
            self.log.push(Event::Outcome { id: outcome.message_id.to_string(), status: outcome.status });
        }
        for message in self.all_pending().await {
            self.log.push(Event::StillPending { id: message.id.to_string(), due: message.deliver_at <= now });
        }
        for to in 0..KEYS.len() {
            let address = script(to).address().expect("a valid address");
            if let Some(record) = self.memory.get(&address).await.expect("the store is up") {
                let state: ScriptState =
                    serde_json::from_value(record.state.unwrap_or(Value::Null)).expect("the state decodes");
                self.log.push(Event::State { address: address.to_string(), ids: state.ids });
            }
        }
    }
}

fn scripts_in(actions: &[Action]) -> u32 {
    actions.iter().map(|a| if let Action::Send { actions, .. } = a { 1 + scripts_in(actions) } else { 0 }).sum()
}

/// How long a drain may take: every script of the scenario, one after the other, at their slowest.
pub fn drain_bound(scenario: &Scenario) -> Duration {
    let scripts: u32 = scenario
        .steps
        .iter()
        .map(|s| if let Step::Send { actions, .. } = s { 1 + scripts_in(actions) } else { 0 })
        .sum();
    let slowest_run = MAX_TURN + 2 * LEASE_TTL + BACKOFF + MAX_FAULT_DELAY;
    slowest_run * (scripts + 1) * (MAX_ATTEMPTS + 2) + MAX_SEND_DELAY * (scripts + 1)
}

/// Runs `scenario` on the current runtime, which must be on paused time, and returns its history.
pub async fn run(scenario: &Scenario) -> Vec<Event> {
    let mut runner = Runner::start(scenario).await;
    for step in &scenario.steps {
        runner.step(step).await;
    }
    runner.drain(drain_bound(scenario)).await;
    runner.record_final_state().await;
    runner.log.events()
}

/// Runs `scenario` on a new runtime on paused time.
pub fn simulate(scenario: &Scenario) -> Vec<Event> {
    let runtime =
        tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().expect("a runtime");
    runtime.block_on(run(scenario))
}

// ---- generators ----

fn action() -> BoxedStrategy<Action> {
    let leaf = prop_oneof![
        3 => prop_oneof![
            3 => 0u64..2_000,
            3 => 2_000u64..6_000,
            1 => 6_000u64..12_000,
        ].prop_map(|ms| Action::Sleep { ms }),
        2 => (0u32..3).prop_map(|times| Action::Fail { times }),
        1 => (0u32..3).prop_map(|times| Action::Panic { times }),
        1 => (0usize..KEYS.len()).prop_map(|to| Action::Create { to }),
        1 => Just(Action::Stop),
    ];
    leaf.prop_recursive(2, 12, 3, |inner| {
        (0usize..KEYS.len(), delay_ms(), retry(), proptest::collection::vec(inner, 0..3))
            .prop_map(|(to, delay_ms, retry, actions)| Action::Send { to, delay_ms, retry, actions })
    })
    .boxed()
}

fn delay_ms() -> impl Strategy<Value = u64> {
    prop_oneof![2 => Just(0u64), 1 => 1u64..(MAX_SEND_DELAY.as_millis() as u64)]
}

fn retry() -> impl Strategy<Value = Option<u32>> {
    prop_oneof![1 => Just(None), 1 => (1..=MAX_ATTEMPTS).prop_map(Some)]
}

fn step() -> BoxedStrategy<Step> {
    let node = || 0usize..3;
    let to = || 0usize..KEYS.len();
    let back = || 0usize..8;
    prop_oneof![
        6 => (0u64..5_000).prop_map(|ms| Step::Advance(Duration::from_millis(ms))),
        6 => (node(), to(), delay_ms(), retry(), proptest::collection::vec(action(), 0..4))
            .prop_map(|(node, to, delay_ms, retry, actions)| Step::Send { node, to, delay_ms, retry, actions }),
        1 => (node(), back()).prop_map(|(node, back)| Step::Resend { node, back }),
        1 => (node(), to()).prop_map(|(node, to)| Step::Create { node, to }),
        1 => (node(), to()).prop_map(|(node, to)| Step::Delete { node, to }),
        1 => (node(), back()).prop_map(|(node, back)| Step::Cancel { node, back }),
        1 => (node(), back()).prop_map(|(node, back)| Step::DeliverNow { node, back }),
        1 => (node(), to(), any::<bool>()).prop_map(|(node, to, paused)| Step::SetPaused { node, to, paused }),
        1 => (node(), to()).prop_map(|(node, to)| Step::Purge { node, to }),
        1 => (node(), to()).prop_map(|(node, to)| Step::RequestCancel { node, to }),
    ]
    .boxed()
}

fn method() -> impl Strategy<Value = Method> {
    prop_oneof![
        4 => Just(Method::Commit),
        4 => Just(Method::Renew),
        3 => Just(Method::ClaimDue),
        1 => proptest::sample::select(Method::ALL.to_vec()),
    ]
}

fn fault() -> impl Strategy<Value = Fault> {
    prop_oneof![
        3 => Just(Fault::Before),
        3 => Just(Fault::After),
        2 => (1u64..(MAX_FAULT_DELAY.as_millis() as u64)).prop_map(|ms| Fault::Delay(Duration::from_millis(ms))),
        2 => Just(Fault::Dead),
    ]
}

/// Mostly the first calls, as a method like `renew` is called only a few times in a run.
fn nth() -> impl Strategy<Value = u32> {
    prop_oneof![3 => 0u32..3, 1 => 3u32..10]
}

fn planned_fault() -> impl Strategy<Value = PlannedFault> {
    (0usize..3, method(), nth(), fault())
        .prop_map(|(node, method, nth, fault)| PlannedFault { node, rule: Rule { method, nth, fault } })
}

/// A whole scenario.
pub fn scenario() -> impl Strategy<Value = Scenario> {
    (1usize..=3, proptest::collection::vec(step(), 1..30), proptest::collection::vec(planned_fault(), 0..10))
        .prop_map(|(nodes, steps, faults)| Scenario { nodes, steps, faults })
}
