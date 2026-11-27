//! The portable model: what the inspector fetches over the HTTP API, how a refresh is bounded, which context it shows,
//! and the sidebar's history. The AppKit view only renders it.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use cicada::ActorAddress;
use cicada::api::{NodeView, RunningTurnView, StatsView};
use cicada::store::MessageId;
use cicada::time::Timestamp;
use cicada_app::{Contexts, Env};
use cicada_client::{ActorClient, ClientError};
use futures::{StreamExt, stream};
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout_at};

use crate::actors::{Actors, OutlineViewport};
use crate::messages::Messages;
use crate::nodes::{NodePlace, Nodes};
use crate::outcomes::{OutcomePlace, Outcomes};
use crate::refresh::{Budget, Done, Readiness, Refresh, RefreshPage, Stale, as_of};
use crate::turns::{RunningTurns, TurnPlace};
use crate::types::Types;
use crate::watches::{WatchPlace, Watches};

/// What the inspector says where it cannot run.
pub const NEEDS_MACOS: &str = "the Cicada inspector requires macOS 14 or later";
/// The running turns the Overview shows: the first page of `/turns`.
pub const TURNS_PAGE: usize = 20;
/// The text of a part whose call had not answered at the deadline.
pub const TIMED_OUT: &str = "no answer within 10 s";
/// The variables that win over the picked context, as they do for `cicada`.
const OVERRIDES: [&str; 2] = ["CICADA_SERVER", "CICADA_TOKEN"];

/// Refuses a macOS before 14, as the tray does.
pub fn check_macos(version: (isize, isize, isize)) -> Result<(), &'static str> {
    if version < (14, 0, 0) { Err(NEEDS_MACOS) } else { Ok(()) }
}

/// The bounds of one refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most types whose actors are counted.
    pub types: usize,
    /// The most per-type counts in flight at once.
    pub concurrent: usize,
    /// How long the whole refresh may take; what has not answered by then is not shown. Each read of the Actors page
    /// has as long.
    pub deadline: Duration,
    /// How long the search field must be still before its prefix is read.
    pub debounce: Duration,
}

impl Limits {
    /// 50 types, 8 at a time, 10 seconds; a search waits 300 ms.
    pub const DEFAULT: Self =
        Self { types: 50, concurrent: 8, deadline: Duration::from_secs(10), debounce: Duration::from_millis(300) };
}

/// One part of the Overview: its value, or why it has none.
#[derive(Debug, Clone, PartialEq)]
pub enum Part<T> {
    /// The call answered.
    Ready(T),
    /// The call failed or did not answer by the deadline: the text to show, without credentials.
    Failed(String),
}

impl<T> Part<T> {
    /// The value mapped by `f`, or the same failure.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Part<U> {
        match self {
            Self::Ready(value) => Part::Ready(f(value)),
            Self::Failed(error) => Part::Failed(error),
        }
    }

    /// The value, if the call answered.
    #[cfg(test)]
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Failed(_) => None,
        }
    }
}

/// The actor count of one type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Count {
    /// `/stats?prefix=<type>/` answered.
    Counted(u64),
    /// The count's call failed: the text to show.
    Failed(String),
    /// Not asked: the type is past the most types counted, or the deadline came first.
    NotCounted,
}

/// A registered type and its actor count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRow {
    /// The type's name.
    pub name: String,
    /// How many actors it has.
    pub actors: Count,
}

/// The first page of the running turns.
#[derive(Debug, Clone, PartialEq)]
pub struct Turns {
    /// At most [`TURNS_PAGE`] turns.
    pub items: Vec<RunningTurnView>,
    /// True when the server has more than this page.
    pub more: bool,
}

/// What the Overview shows, each part with its own result.
#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    /// `/stats` of the whole store.
    pub stats: Part<StatsView>,
    /// `/health`: true when the node and its store answer.
    pub health: Part<bool>,
    /// `/nodes`.
    pub nodes: Part<Vec<NodeView>>,
    /// The first page of `/turns`.
    pub turns: Part<Turns>,
    /// `/types`, with each type's actor count.
    pub types: Part<Vec<TypeRow>>,
}

/// Gathers an Overview within `limits`: the base routes at once, and once `/types` answers, the actor counts of the
/// first types, a bounded number at a time. At the deadline the answers that came are kept; a base route still out
/// fails with [`TIMED_OUT`], and a type still uncounted is [`Count::NotCounted`].
#[cfg(test)]
pub async fn overview(client: &ActorClient, limits: Limits) -> Overview {
    overview_with(client, limits, |_| {}).await
}

/// [`overview`], handing `/stats` to `on_stats` as soon as it answers, before the other routes have.
pub(crate) async fn overview_with(
    client: &ActorClient,
    limits: Limits,
    on_stats: impl FnOnce(&Part<StatsView>),
) -> Overview {
    let budget = Budget::with_slots(limits.deadline, limits.concurrent.max(1).min(limits.types.max(1)) + 4);
    gather_overview(client, limits, &budget, on_stats).await
}

async fn gather_overview(
    client: &ActorClient,
    limits: Limits,
    budget: &Budget,
    on_stats: impl FnOnce(&Part<StatsView>),
) -> Overview {
    let turns = async {
        let page = client.turns(None, "", None, Some(TURNS_PAGE)).await?;
        Ok(Turns { more: page.next.is_some(), items: page.items })
    };
    // The counts start as soon as `/types` answers, not after the other base routes.
    let types = async {
        match budget.read(client.types()).await {
            Part::Ready(types) => {
                Part::Ready(count(client, types.into_iter().map(|t| t.name).collect(), budget, limits).await)
            }
            Part::Failed(error) => Part::Failed(error),
        }
    };
    let stats = async {
        let stats = budget.read(client.stats("")).await;
        on_stats(&stats);
        stats
    };
    let (stats, health, nodes, turns, types) =
        tokio::join!(stats, budget.read(client.health()), budget.read(client.nodes()), budget.read(turns), types,);
    Overview { stats, health, nodes, turns, types }
}

/// The result of `call`, or [`TIMED_OUT`] when it has not answered by `deadline`.
pub(crate) async fn within<T>(deadline: Instant, call: impl Future<Output = Result<T, ClientError>>) -> Part<T> {
    match timeout_at(deadline, call).await {
        Ok(Ok(value)) => Part::Ready(value),
        Ok(Err(error)) => Part::Failed(error_text(&error)),
        Err(_) => Part::Failed(TIMED_OUT.into()),
    }
}

/// The actor counts of the first `limits.types` of `names`, in their order, at most `limits.concurrent` at a time.
async fn count(client: &ActorClient, names: Vec<String>, budget: &Budget, limits: Limits) -> Vec<TypeRow> {
    count_rows(names, limits, |name| async move {
        let prefix = format!("{name}/");
        budget.read(client.stats(&prefix)).await.map(|stats| stats.actors)
    })
    .await
}

/// Runs a simulated count under the same deadline for the portable Overview tests.
#[cfg(test)]
pub(crate) async fn count_with<F, Fut>(
    names: Vec<String>,
    deadline: Instant,
    limits: Limits,
    count_one: F,
) -> Vec<TypeRow>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<u64, ClientError>>,
{
    count_rows(names, limits, |name| async {
        if Instant::now() >= deadline {
            return Part::Failed(TIMED_OUT.into());
        }
        within(deadline, count_one(name)).await
    })
    .await
}

async fn count_rows<F, Fut>(names: Vec<String>, limits: Limits, count_one: F) -> Vec<TypeRow>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Part<u64>>,
{
    let asked = names.len().min(limits.types);
    let count_one = &count_one;
    let counts: Vec<Count> = stream::iter(names[..asked].iter().cloned().map(|name| async move {
        match count_one(name).await {
            Part::Ready(actors) => Count::Counted(actors),
            Part::Failed(error) if error == TIMED_OUT => Count::NotCounted,
            Part::Failed(error) => Count::Failed(error),
        }
    }))
    .buffered(limits.concurrent.max(1))
    .collect()
    .await;
    let counts = counts.into_iter().chain(std::iter::repeat(Count::NotCounted));
    names.into_iter().zip(counts).map(|(name, actors)| TypeRow { name, actors }).collect()
}

/// `error` and its causes, each only when the text so far does not hold it. It holds no credentials of the server
/// URL: the client never has them ([`cicada_app::context_client`]).
pub fn error_text(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text = format!("{text}: {cause_text}");
        }
        source = cause.source();
    }
    text
}

/// What the window shows: the contexts, the target in use and the latest Overview.
#[derive(Debug, Clone, Default)]
pub struct View {
    /// The contexts of the config file.
    pub contexts: Contexts,
    /// Why the contexts could not be read.
    pub contexts_error: Option<String>,
    /// The context the user picked; `None` until they pick one.
    pub picked: Option<String>,
    /// The context in use, as `cicada` resolves it.
    pub context: Option<String>,
    /// The variables set in the environment that win over the picked context.
    pub overrides: Vec<&'static str>,
    /// Why no client could be made for the context; the window stays usable.
    pub target_error: Option<String>,
    /// The latest Overview of the context in use, if one finished.
    pub overview: Option<Overview>,
    /// True while a refresh runs.
    pub refreshing: bool,
    /// The mark on parts a soft refresh could not update.
    pub stale: Option<Stale>,
}

impl View {
    /// The subtitle's target, including a server configured without a named context.
    pub fn target_label(&self) -> &str {
        self.context.as_deref().or(self.picked.as_deref()).unwrap_or(if self.target_error.is_some() {
            "No context"
        } else {
            "Server"
        })
    }
}

/// The context in use and its refreshes, publishing each [`View`] through a `watch`. A new refresh, or a switch of
/// context, cancels the one before; a refresh that still finishes after that is discarded.
pub struct Session {
    env: Env,
    shared: Arc<Shared>,
    actors: Actors,
    messages: Messages,
    turns: RunningTurns,
    nodes: Nodes,
    types: Types,
    watches: Watches,
}

struct Shared {
    handle: Handle,
    limits: Limits,
    outcomes: Outcomes,
    view: watch::Sender<View>,
    current: Mutex<Current>,
}

#[derive(Default)]
struct Current {
    /// Grows with every refresh and every switch of context; a refresh publishes only while it is its own.
    generation: u64,
    client: Option<ActorClient>,
    /// Grows with every context picked.
    context: u64,
    task: Option<JoinHandle<()>>,
    refresh: Refresh,
    read_at: Option<Timestamp>,
    /// The refreshes cancelled so far, for tests that await their end.
    #[cfg(test)]
    superseded: Vec<JoinHandle<()>>,
}

impl Session {
    /// A session with no context yet; [`Self::use_context`] picks one. Refreshes run on `handle`.
    pub fn new(env: Env, handle: Handle, limits: Limits) -> Self {
        let shared = Arc::new(Shared {
            handle: handle.clone(),
            limits,
            outcomes: Outcomes::new(handle.clone(), limits),
            view: watch::channel(View::default()).0,
            current: Mutex::default(),
        });
        let actors = Actors::new(handle.clone(), limits);
        let messages = Messages::new(handle.clone(), limits);
        let turns = RunningTurns::new(handle.clone(), limits);
        let nodes = Nodes::new(handle.clone(), limits);
        let types = Types::new(handle.clone(), limits);
        let watches = Watches::new(handle.clone(), limits);
        Self { env, shared, actors, messages, turns, nodes, types, watches }
    }

    /// The Actors page's reads, on the client of the context in use.
    pub fn actors(&self) -> &Actors {
        &self.actors
    }

    /// The Messages page's reads, on the client of the context in use.
    pub fn messages(&self) -> &Messages {
        &self.messages
    }

    /// The Running Turns page's reads, on the client of the context in use.
    pub fn turns(&self) -> &RunningTurns {
        &self.turns
    }

    /// The Outcomes page and the sidebar's failed count, on the client of the context in use.
    pub fn outcomes(&self) -> &Outcomes {
        &self.shared.outcomes
    }

    /// The Nodes page's cache and pane, on the client of the context in use; the Overview's refresh leaves it alone.
    pub fn nodes(&self) -> &Nodes {
        &self.nodes
    }

    /// The Types page on the client of the context in use.
    pub fn types(&self) -> &Types {
        &self.types
    }

    /// The Watches page on the client of the context in use.
    pub fn watches(&self) -> &Watches {
        &self.watches
    }

    /// The views, newest last.
    pub fn view(&self) -> watch::Receiver<View> {
        self.shared.view.subscribe()
    }

    /// Reads the contexts again, makes the client for `picked` (`None`: as `cicada` picks one) and refreshes. The
    /// Overview and the lists and panes of the context before are dropped.
    pub fn use_context(&self, picked: Option<String>) {
        let contexts = Contexts::load(&self.env);
        let target = cicada_app::context_client(&self.env, picked.as_deref());
        let overrides = OVERRIDES.into_iter().filter(|name| self.env.get(*name).is_some_and(|v| !v.is_empty()));
        {
            let mut current = self.shared.current.lock().expect("inspector session");
            current.supersede();
            current.context += 1;
            self.shared.view.send_modify(|view| {
                (view.contexts, view.contexts_error) = match contexts {
                    Ok(contexts) => (contexts, None),
                    Err(error) => (Contexts::default(), Some(error_text(error.as_ref()))),
                };
                view.picked = picked;
                view.overrides = overrides.collect();
                view.overview = None;
                view.stale = None;
                view.refreshing = false;
                match target {
                    Ok((client, context)) => {
                        view.context = context;
                        view.target_error = None;
                        current.client = Some(client);
                    }
                    Err(error) => {
                        view.context = None;
                        view.target_error = Some(error_text(error.as_ref()));
                        current.client = None;
                    }
                }
            });
            self.actors.use_client(current.client.clone());
            self.messages.use_client(current.client.clone());
            self.turns.use_client(current.client.clone());
            self.shared.outcomes.use_client(current.client.clone());
            self.nodes.use_client(current.client.clone());
            self.types.use_client(current.client.clone());
            self.watches.use_client(current.client.clone());
        }
        self.refresh();
    }

    /// Starts a refresh of the context in use, cancelling one that runs; its `/stats` also gives the sidebar's failed
    /// count as soon as it answers, unless a later refresh or a context switch came first. Without a client it does
    /// nothing.
    pub fn refresh(&self) {
        let mut current = self.shared.current.lock().expect("inspector session");
        current.supersede();
        let Some(client) = current.client.clone() else { return };
        let generation = current.generation;
        self.shared.view.send_modify(|view| {
            view.refreshing = true;
            view.stale = None;
        });
        let (shared, limits, count) = (self.shared.clone(), self.shared.limits, self.shared.outcomes.count_ticket());
        let counted = shared.clone();
        current.task = Some(self.shared.handle.spawn(async move {
            let land = move |stats: &Part<StatsView>| {
                let current = counted.current.lock().expect("inspector session");
                if current.generation == generation {
                    count.land(stats);
                }
            };
            let overview = overview_with(&client, limits, land).await;
            let mut current = shared.current.lock().expect("inspector session");
            if current.generation == generation {
                current.task = None;
                current.read_at = Some(chrono::Utc::now());
                shared.view.send_modify(|view| {
                    view.overview = Some(overview);
                    view.refreshing = false;
                });
                shared.resume(&mut current);
            }
        }));
    }

    /// Re-reads a page without clearing its cached rows or selection; [`Done`] when the attempt ends. Without a
    /// `viewport`, the Actors page uses the one the view last reported.
    pub(crate) fn soft_refresh(&self, section: Section, viewport: Option<OutlineViewport>) -> Done {
        match section {
            Section::Overview => self.shared.request(),
            Section::Actors => self.actors.soft_refresh(viewport.unwrap_or_else(|| self.actors.viewport())),
            Section::Messages => self.messages.soft_refresh(),
            Section::RunningTurns => self.turns.soft_refresh(),
            Section::Outcomes => self.shared.outcomes.soft_refresh(),
            Section::Nodes => self.nodes.soft_refresh(),
            Section::Types => self.types.soft_refresh(),
            Section::Watches => self.watches.soft_refresh(),
        }
    }

    /// Cancels a page's soft refresh and drops its results, the page staying as it is.
    pub(crate) fn cancel_live_refresh(&self, section: Section) {
        match section {
            Section::Nodes => self.nodes.cancel_soft_refresh(),
            _ => self.cancel_soft_refresh(section),
        }
    }

    /// The actor a page's pane shows, if any.
    pub(crate) fn pane_actor(&self, section: Section) -> Option<ActorAddress> {
        match section {
            Section::Actors => self.actors.pane_actor(),
            Section::Messages => self.messages.pane_actor(),
            Section::RunningTurns => self.turns.pane_actor(),
            Section::Outcomes => self.shared.outcomes.pane_actor(),
            _ => None,
        }
    }

    /// The client of the context in use, and a number that grows with every [`Self::use_context`].
    pub fn client(&self) -> (u64, Option<ActorClient>) {
        let current = self.shared.current.lock().expect("inspector session");
        (current.context, current.client.clone())
    }

    fn cancel_soft_refresh(&self, section: Section) {
        match section {
            Section::Overview => self.shared.lock_refresh().refresh.cancel(),
            Section::Actors => self.actors.cancel_soft_refresh(),
            Section::Messages => self.messages.cancel_soft_refresh(),
            Section::RunningTurns => self.turns.cancel_soft_refresh(),
            Section::Outcomes => self.shared.outcomes.cancel_soft_refresh(),
            Section::Nodes => self.nodes.hide(),
            Section::Types => self.types.cancel_soft_refresh(),
            Section::Watches => self.watches.cancel_soft_refresh(),
        }
    }

    /// The refreshes cancelled so far; each ends once it has stopped, with or without its own end.
    #[cfg(test)]
    pub fn take_superseded(&self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.shared.current.lock().expect("inspector session").superseded)
    }

    /// Picks a context as [`Self::use_context`] does and forgets `navigation`'s history, whose places belong to the
    /// context before.
    pub fn switch_context(&self, navigation: &mut Navigation, picked: Option<String>) {
        self.use_context(picked);
        navigation.forget();
    }

    /// Applies `change` to the history, remembering the Messages, Running Turns, Outcomes, Nodes or Watches place left
    /// and restoring the place reached. Leaving Actors cancels a pending Show Actor, and arriving at a Show Actor place
    /// starts it again. Returns what `change` returned.
    pub fn navigate(&self, navigation: &mut Navigation, change: impl FnOnce(&mut Navigation) -> bool) -> bool {
        let left = navigation.current();
        let recorded = navigation.place().clone();
        match left {
            Section::Messages => navigation.current.messages = Some(self.messages.source()),
            Section::RunningTurns => navigation.current.turns = Some(self.turns.source()),
            Section::Nodes => navigation.current.nodes = Some(self.nodes.source()),
            // What the pane shows is kept; but a link place whose pane a status switch or a context switch emptied
            // keeps its message, read fresh on every return.
            Section::Outcomes => {
                let source = self.shared.outcomes.source();
                let link = matches!(navigation.current.outcomes, Some(OutcomePlace::Linked(_)));
                if !(link && source == OutcomePlace::Nothing) {
                    navigation.current.outcomes = Some(source);
                }
            }
            Section::Watches => navigation.current.watches = Some(self.watches.source()),
            _ => {}
        }
        if !change(navigation) {
            navigation.current = recorded;
            return false;
        }
        if left != navigation.current() {
            self.cancel_soft_refresh(left);
        }
        if left == Section::Actors {
            self.actors.cancel_jump();
        }
        let place = navigation.place();
        if let Some(actor) = &place.actor {
            self.actors.jump(actor.clone());
        }
        if let Some((prefix, selected)) = &place.messages {
            self.messages.restore(prefix, selected.clone());
        }
        if let Some(watch) = &place.watches {
            self.watches.restore(watch.clone());
        }
        if let Some(outcome) = &place.outcomes {
            self.shared.outcomes.restore(outcome.clone());
        }
        if let Some(turn) = &place.turns {
            self.turns.restore(turn.clone());
        }
        if let Some(node) = &place.nodes {
            self.nodes.restore(node.clone());
        }
        true
    }

    /// Follows a link of the cause chain: records a place of the Outcomes page that reads the message `id` fresh, and
    /// reads it.
    pub fn follow(&self, navigation: &mut Navigation, id: MessageId) {
        self.navigate(navigation, |navigation| {
            navigation.link(id);
            true
        });
    }

    /// Show Actor: records a jump to `actor` on the Actors page and starts it.
    pub fn show_actor(&self, navigation: &mut Navigation, actor: ActorAddress) {
        self.navigate(navigation, |navigation| {
            navigation.jump(actor);
            true
        });
    }

    /// Cancels a refresh and the pages' reads, and discards their results; for a window that closes.
    pub fn stop(&self) {
        self.shared.current.lock().expect("inspector session").supersede();
        self.actors.stop();
        self.messages.stop();
        self.turns.stop();
        self.shared.outcomes.stop();
        self.nodes.stop();
        self.types.stop();
        self.watches.stop();
        self.shared.view.send_modify(|view| view.refreshing = false);
    }
}

impl RefreshPage for Shared {
    type State = Current;

    fn lock_refresh(&self) -> MutexGuard<'_, Current> {
        self.current.lock().expect("inspector session")
    }

    fn refresh(state: &mut Current) -> &mut Refresh {
        &mut state.refresh
    }

    fn runtime(&self) -> (&Handle, Duration) {
        (&self.handle, self.limits.deadline)
    }

    fn readiness(&self, state: &Current) -> Readiness {
        if state.task.is_some() {
            Readiness::Wait
        } else if state.client.is_some() {
            Readiness::Ready
        } else {
            Readiness::Skip
        }
    }

    async fn run(self: &Arc<Self>, ticket: u64, budget: Budget) {
        let (client, count) = {
            let state = self.lock_refresh();
            if !state.refresh.current(ticket) {
                return;
            }
            let Some(client) = state.client.clone() else { return };
            (client, self.outcomes.count_ticket())
        };
        let mut fresh = gather_overview(&client, self.limits, &budget, |_| {}).await;
        let mut state = self.lock_refresh();
        if !state.refresh.current(ticket) {
            return;
        }
        count.land(&fresh.stats);
        let mut errors = Vec::new();
        self.view.send_modify(|view| {
            let as_of = as_of(view.stale.as_ref(), state.read_at);
            let old = view.overview.take();
            retain_part(&mut fresh.stats, old.as_ref().map(|old| &old.stats), &mut errors);
            retain_part(&mut fresh.health, old.as_ref().map(|old| &old.health), &mut errors);
            retain_part(&mut fresh.nodes, old.as_ref().map(|old| &old.nodes), &mut errors);
            retain_part(&mut fresh.turns, old.as_ref().map(|old| &old.turns), &mut errors);
            let old_types = old.as_ref().map(|old| &old.types);
            if let Part::Ready(rows) = &mut fresh.types {
                retain_counts(rows, old_types, &budget, self.limits, &mut errors);
            } else {
                retain_part(&mut fresh.types, old_types, &mut errors);
            }
            view.overview = Some(fresh);
            view.stale = (!errors.is_empty()).then(|| Stale::failed(as_of, errors.join("\n")));
        });
        state.read_at = Some(chrono::Utc::now());
    }
}

fn retain_part<T: Clone>(fresh: &mut Part<T>, previous: Option<&Part<T>>, errors: &mut Vec<String>) {
    if let Part::Failed(error) = fresh {
        if !errors.contains(error) {
            errors.push(error.clone());
        }
        if let Some(previous @ Part::Ready(_)) = previous {
            *fresh = previous.clone();
        }
    }
}

fn retain_counts(
    rows: &mut [TypeRow],
    old: Option<&Part<Vec<TypeRow>>>,
    budget: &Budget,
    limits: Limits,
    errors: &mut Vec<String>,
) {
    let old = match old {
        Some(Part::Ready(old)) => old.as_slice(),
        _ => &[],
    };
    for row in rows.iter_mut().take(limits.types) {
        let error = match &row.actors {
            Count::Failed(error) => error,
            Count::NotCounted if Instant::now() >= budget.deadline() => TIMED_OUT,
            _ => continue,
        };
        if !errors.iter().any(|before| before == error) {
            errors.push(error.to_owned());
        }
        if let Some(before) =
            old.iter().find(|before| before.name == row.name && matches!(before.actors, Count::Counted(_)))
        {
            row.actors = before.actors.clone();
        }
    }
}

impl Current {
    fn supersede(&mut self) {
        self.generation += 1;
        self.refresh.cancel();
        self.read_at = None;
        if let Some(task) = self.task.take() {
            task.abort();
            #[cfg(test)]
            self.superseded.push(task);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(mut current) = self.shared.current.lock() {
            current.supersede();
        }
    }
}

/// An item of the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Section {
    /// The Overview.
    #[default]
    Overview,
    /// Actors (`inspector-actors`).
    Actors,
    /// Pending messages (`inspector-messages`).
    Messages,
    /// Running turns (`inspector-nodes`).
    RunningTurns,
    /// Finished messages (`inspector-outcomes`).
    Outcomes,
    /// Nodes (`inspector-nodes`).
    Nodes,
    /// Types.
    Types,
    /// Watches.
    Watches,
}

impl Section {
    /// The items under Browse, in the sidebar's order.
    pub const BROWSE: [Self; 7] =
        [Self::Actors, Self::Messages, Self::RunningTurns, Self::Outcomes, Self::Nodes, Self::Types, Self::Watches];

    /// The sidebar's text.
    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Actors => "Actors",
            Self::Messages => "Messages",
            Self::RunningTurns => "Running Turns",
            Self::Outcomes => "Outcomes",
            Self::Nodes => "Nodes",
            Self::Types => "Types",
            Self::Watches => "Watches",
        }
    }
}

/// A place in the history: a sidebar item, with a Show Actor destination, a Messages, Running Turns, Nodes or Watches
/// selection, or what the Outcomes pane showed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place {
    /// The item shown.
    pub section: Section,
    /// The actor a Show Actor selects on the Actors page, again whenever back or forward return here.
    pub actor: Option<ActorAddress>,
    /// The Messages page's prefix and selected message when this place was left, restored on a return to it.
    pub messages: Option<(String, Option<MessageId>)>,
    /// The Watches page's filter and selection when this place was left.
    pub watches: Option<WatchPlace>,
    /// What the Outcomes pane showed when this place was left, or the message a link of the cause chain reads.
    pub outcomes: Option<OutcomePlace>,
    /// The Running Turns page's prefix and the turn its pane showed, as read, when this place was left.
    pub turns: Option<TurnPlace>,
    /// The Nodes page's filter and selected node when this place was left.
    pub nodes: Option<NodePlace>,
}

/// The sidebar's selection with a back and forward history, as in a browser.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Navigation {
    back: Vec<Place>,
    current: Place,
    forward: Vec<Place>,
}

impl Navigation {
    /// The selected item.
    pub fn current(&self) -> Section {
        self.current.section
    }

    /// The current place.
    pub fn place(&self) -> &Place {
        &self.current
    }

    /// Selects `section`; a new selection drops the forward list. Selecting the current item changes nothing.
    pub fn select(&mut self, section: Section) {
        if section != self.current.section {
            self.go(Place { section, ..Place::default() });
        }
    }

    /// Records a Show Actor: the Actors page with `actor` as its target, after the current place.
    pub fn jump(&mut self, actor: ActorAddress) {
        self.go(Place { section: Section::Actors, actor: Some(actor), ..Place::default() });
    }

    /// Records a link of the cause chain: the Outcomes page showing the message `id`, after the current place.
    pub fn link(&mut self, id: MessageId) {
        self.go(Place { section: Section::Outcomes, outcomes: Some(OutcomePlace::Linked(id)), ..Place::default() });
    }

    /// Forgets the history and what the current place restores: after a context switch, a place of the context
    /// before would jump to, or restore, a same-named actor or message of another server.
    pub fn forget(&mut self) {
        self.back.clear();
        self.forward.clear();
        self.current = Place { section: self.current.section, ..Place::default() };
    }

    fn go(&mut self, place: Place) {
        self.back.push(std::mem::replace(&mut self.current, place));
        self.forward.clear();
    }

    /// Goes back one selection; false when there is none.
    pub fn back(&mut self) -> bool {
        let Some(place) = self.back.pop() else { return false };
        self.forward.push(std::mem::replace(&mut self.current, place));
        true
    }

    /// Goes forward one selection; false when there is none.
    pub fn forward(&mut self) -> bool {
        let Some(place) = self.forward.pop() else { return false };
        self.back.push(std::mem::replace(&mut self.current, place));
        true
    }

    /// True when [`Self::back`] can go back.
    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// True when [`Self::forward`] can go forward.
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

/// A tile of the Overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// What it counts.
    pub label: &'static str,
    /// The number or text, `—` when there is none.
    pub value: String,
    /// The small text under the value: what it means, or the error of its call.
    pub note: String,
    /// True when `note` is an error.
    pub error: bool,
}

/// The four tiles of `/stats`: actors, pending, oldest due and failed (kept by retention, which the API does not
/// report). A failed call shows its error on each; a `failed` an older server omits shows `—`.
pub fn tiles(stats: &Part<StatsView>, now: Timestamp) -> [Tile; 4] {
    let tile = |label, value: String, note: &str| Tile { label, value, note: note.to_owned(), error: false };
    match stats {
        Part::Ready(stats) => [
            tile("Actors", stats.actors.to_string(), "in the store"),
            tile("Pending", stats.pending.to_string(), "messages, delayed ones included"),
            match stats.oldest_due {
                Some(due) => tile("Oldest due", age(due, now), "waiting since"),
                None => tile("Oldest due", "—".into(), "nothing due"),
            },
            tile("Failed", stats.failed.map_or_else(|| "—".to_owned(), |n| n.to_string()), "kept by retention"),
        ],
        Part::Failed(error) => ["Actors", "Pending", "Oldest due", "Failed"].map(|label| Tile {
            label,
            value: "—".into(),
            note: error.clone(),
            error: true,
        }),
    }
}

/// How long ago `instant` was, short: `42s`, `5m`, `3h`, `2d`; `in 1m` for one ahead.
pub fn age(instant: Timestamp, now: Timestamp) -> String {
    let seconds = (now - instant).num_seconds();
    let n = seconds.unsigned_abs();
    let short = match n {
        0..60 => format!("{n}s"),
        60..3600 => format!("{}m", n / 60),
        3600..172_800 => format!("{}h", n / 3600),
        _ => format!("{}d", n / 86_400),
    };
    if seconds < 0 { format!("in {short}") } else { short }
}

/// The text of a type's count: the number, `not counted`, or `error`.
pub fn count_text(count: &Count) -> String {
    match count {
        Count::Counted(n) => n.to_string(),
        Count::Failed(_) => "error".into(),
        Count::NotCounted => "not counted".into(),
    }
}
