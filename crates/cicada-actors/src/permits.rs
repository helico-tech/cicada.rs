//! The permit pool (ADR 0058): an actor that lets at most its capacity of requests hold a permit at once, queues the
//! rest in FIFO order, and fences each grant with a number that only goes up.

use std::collections::HashSet;

use cicada::prelude::*;
use cicada::runtime::limits::MAX_TYPE_LENGTH;
use cicada::turn::TurnError;
use cicada::{Failed, Stopped};
use serde::{Deserialize, Serialize};

use crate::supervisor::retry_policy;

/// The largest capacity and queue bound of a pool.
const MAX_SIZE: u32 = 1_000;
/// The longest request ID, in characters.
const MAX_REQUEST_LENGTH: usize = 200;

/// A permit pool, `cicada.permits/<pool>`: one for each capacity it guards.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Permits {
    /// The name of the pool: the key of its address.
    pub pool: String,
}

impl Permits {
    /// The pool named `pool`.
    pub fn named(pool: impl Into<String>) -> Self {
        Self { pool: pool.into() }
    }
}

impl ActorRef for Permits {
    const TYPE: &'static str = "cicada.permits";
    type Message = PermitsMsg;
}

/// The messages of a [`Permits`] pool. `Notify` is its own: it acts on one only when it sent it itself. A request ID is
/// not empty, at most 200 characters, and holds no NUL; a message with one out of bounds is refused for good.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum PermitsMsg {
    /// Asks one permit for `request`, and names where its notices go. With room it grants at once; otherwise the request
    /// waits in FIFO order; with the queue full it is refused and nothing is recorded. The answer, `granted`, `queued` or
    /// `refused`, goes to the target in this commit, so a target that is gone rejects it and nothing is recorded. A
    /// repeated `Acquire` of a holder answers `granted` with the same number, one of a waiting request `queued` with its
    /// place kept; both keep the target of the first. A request ID is forgotten at its release: use a fresh one per
    /// attempt.
    Acquire {
        /// The request: names this attempt.
        #[schemars(length(min = 1, max = 200))]
        request: String,
        /// The target of the notices.
        to: ActorAddress,
        /// The message type of the notices, whose payload is a [`PermitNotice`]; not empty, at most 200 characters,
        /// without NUL.
        #[schemars(length(min = 1, max = 200))]
        message_type: String,
    },
    /// Frees the permit of `request` when it holds `grant`; ignored otherwise, so a late release of an earlier grant
    /// frees nothing.
    Release {
        /// The request.
        #[schemars(length(min = 1, max = 200))]
        request: String,
        /// The grant number it releases.
        grant: u64,
    },
    /// Takes a waiting request out of the queue; ignored for a holder, which releases with its grant number instead.
    Cancel {
        /// The request.
        #[schemars(length(min = 1, max = 200))]
        request: String,
    },
    /// Sets the capacity, 0 through 1,000; 0 pauses the pool. A growth grants to the head of the queue; a shrink
    /// revokes nothing, and new grants wait until fewer hold than the capacity.
    Resize {
        /// The new capacity.
        #[schemars(range(max = 1000))]
        capacity: u32,
    },
    /// Sends a `check` notice to every request in the books, holders with their grant number and waiting requests
    /// without, each in a turn of its own. A holder whose `granted` notice has not committed yet gets none: its grant
    /// is on its way, and the next audit checks it; so a grant number reaches a holder only through `granted`. It frees
    /// nothing: a holder whose work is confirmed stopped answers with `Release`, a waiting request that gave up with
    /// `Cancel`.
    Audit,
    /// Sends `request` what is true when it runs: its `granted` notice, or with `check` a `check` notice; nothing for a
    /// request no longer in the books, and no `check` to a holder not yet told of its grant. Sent by the pool to itself,
    /// with a retry policy.
    Notify {
        /// The request.
        #[schemars(length(min = 1, max = 200))]
        request: String,
        /// A `check` of an audit; false: a grant.
        check: bool,
    },
    /// A message of the pool itself ended `failed`: a grant `Notify` that failed for a holder that never got its
    /// `granted` notice frees that permit, as the holder never learned its number. Only a notification that the store
    /// wrote counts.
    Failed(Failed),
    /// Ignored.
    Stopped(Stopped),
}

impl PermitsMsg {
    /// An `Acquire` of one permit for `request`, whose notices go to `holder` as `message_type`. Fails for a ref without
    /// an address, a request ID that is empty, longer than 200 characters or holds a NUL, and a message type that is
    /// empty, longer than `MAX_TYPE_LENGTH` characters or holds a NUL. Whether the holder's type takes the message type
    /// is checked at the send.
    ///
    /// ```
    /// # use cicada::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Debug, Clone, Serialize, Deserialize)]
    /// # struct Build { id: String }
    /// # impl ActorRef for Build { const TYPE: &'static str = "build"; type Message = BuildMsg; }
    /// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// # #[serde(tag = "type")]
    /// # enum BuildMsg { Permit(cicada_actors::PermitNotice) }
    /// use cicada_actors::PermitsMsg;
    ///
    /// let acquire = PermitsMsg::acquire("b1-attempt-1", &Build { id: "b1".into() }, "Permit")?;
    /// assert_eq!(serde_json::to_value(&acquire)?, serde_json::json!({"type": "Acquire",
    ///     "request": "b1-attempt-1", "to": "build/b1", "messageType": "Permit"}));
    /// assert!(PermitsMsg::acquire("", &Build { id: "b1".into() }, "Permit").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn acquire<R: ActorRef>(
        request: impl Into<String>,
        holder: &R,
        message_type: impl Into<String>,
    ) -> Result<Self, InvalidPermits> {
        let to = holder.address().map_err(|e| InvalidPermits(e.to_string()))?;
        let (request, message_type) = (request.into(), message_type.into());
        check_request(&request)?;
        check_message_type(&message_type)?;
        Ok(PermitsMsg::Acquire { request, to, message_type })
    }
}

/// What a notice of a pool says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PermitNoticeKind {
    /// The request holds a permit, with its grant number.
    Granted,
    /// The request waits in the queue.
    Queued,
    /// The queue was full: nothing was recorded.
    Refused,
    /// An audit asks the request to check in: a holder whose work is confirmed stopped releases, a waiting request
    /// that gave up cancels, one still working ignores it.
    Check,
}

/// A notice of a pool to the target of a request, sent as the message type of its `Acquire`. A host's family takes it
/// as a variant such as `Permit(PermitNotice)`.
///
/// On the wire: `{"pool":"builds","request":"b1-attempt-1","kind":"granted","grant":4}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PermitNotice {
    /// The pool: the key of its address.
    pub pool: String,
    /// The request it is about.
    pub request: String,
    /// What it says.
    pub kind: PermitNoticeKind,
    /// The grant number, set for a holder: pass it to the guarded resource, and name it in the `Release`.
    #[serde(default)]
    pub grant: Option<u64>,
}

/// A holder of a permit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PermitHolder {
    /// The request; not empty, at most 200 characters, without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub request: String,
    /// The target of its notices.
    pub to: ActorAddress,
    /// The message type of its notices; not empty, at most 200 characters, without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub message_type: String,
    /// Its grant number: at least 1, below the pool's `nextGrant`.
    #[schemars(range(min = 1))]
    pub grant: u64,
    /// True once a `granted` notice to it committed; while false, a failed grant `Notify` frees the permit.
    pub notified: bool,
}

/// A request waiting for a permit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WaitingRequest {
    /// The request; not empty, at most 200 characters, without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub request: String,
    /// The target of its notices.
    pub to: ActorAddress,
    /// The message type of its notices; not empty, at most 200 characters, without NUL.
    #[schemars(length(min = 1, max = 200))]
    pub message_type: String,
}

/// A state or a message of a pool out of its bounds; it prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidPermits(String);

/// The state of a [`Permits`] pool: its capacity and queue bound, given at its create, its holders in the order they
/// were granted, its waiting requests in FIFO order, and the next grant number. Build it with `PermitsState::new`; a
/// decoded state is checked the same way.
///
/// On the wire: `{"capacity":4,"maxWaiting":100,"nextGrant":1,"holders":[],"waiting":[]}`; `nextGrant`, `holders` and
/// `waiting` may be left out of a create.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", try_from = "RawState")]
pub struct PermitsState {
    /// The most holders that are granted at once; after a shrink more may hold until they release.
    pub capacity: u32,
    /// The most waiting requests; fixed at the create.
    pub max_waiting: u32,
    /// The number of the next grant; it only goes up.
    pub next_grant: u64,
    /// The holders, in the order they were granted.
    pub holders: Vec<PermitHolder>,
    /// The waiting requests, in FIFO order.
    pub waiting: Vec<WaitingRequest>,
}

/// The wire form of a [`PermitsState`], unchecked: its schema is the contract's.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RawState {
    /// The most holders that are granted at once; 0 through 1000, 0 pauses the pool. After a shrink more may hold
    /// until they release.
    #[schemars(range(max = 1000))]
    capacity: u32,
    /// The most waiting requests; 0 through 1000.
    #[schemars(range(max = 1000))]
    max_waiting: u32,
    /// The number of the next grant, at least 1; it only goes up. Absent: 1.
    #[serde(default = "first_grant")]
    #[schemars(range(min = 1))]
    next_grant: u64,
    /// The holders, in the order they were granted, each request ID once in the books; absent: none.
    #[serde(default)]
    holders: Vec<PermitHolder>,
    /// The waiting requests, in FIFO order, at most `maxWaiting`, each request ID once in the books; absent: none.
    #[serde(default)]
    waiting: Vec<WaitingRequest>,
}

fn first_grant() -> u64 {
    1
}

impl TryFrom<RawState> for PermitsState {
    type Error = InvalidPermits;

    fn try_from(raw: RawState) -> Result<Self, InvalidPermits> {
        let state = Self {
            capacity: raw.capacity,
            max_waiting: raw.max_waiting,
            next_grant: raw.next_grant,
            holders: raw.holders,
            waiting: raw.waiting,
        };
        state.check()?;
        Ok(state)
    }
}

impl PermitsState {
    /// A pool that grants at most `capacity` permits at once and queues at most `max_waiting` requests, with no holders,
    /// no waiting requests and next grant number 1. Fails unless both are 0 through 1,000; a capacity of 0 pauses the
    /// pool.
    ///
    /// ```
    /// use cicada_actors::PermitsState;
    ///
    /// let state = PermitsState::new(4, 100)?;
    /// assert_eq!(serde_json::to_value(&state)?, serde_json::json!({
    ///     "capacity": 4, "maxWaiting": 100, "nextGrant": 1, "holders": [], "waiting": []}));
    /// assert!(PermitsState::new(1001, 100).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(capacity: u32, max_waiting: u32) -> Result<Self, InvalidPermits> {
        let state = Self { capacity, max_waiting, next_grant: 1, holders: Vec::new(), waiting: Vec::new() };
        state.check()?;
        Ok(state)
    }

    fn check(&self) -> Result<(), InvalidPermits> {
        check_capacity(self.capacity)?;
        if self.max_waiting > MAX_SIZE {
            return Err(InvalidPermits(format!("maxWaiting must be 0 through {MAX_SIZE}, was {}", self.max_waiting)));
        }
        if self.next_grant == 0 {
            return Err(InvalidPermits("nextGrant must be at least 1".to_owned()));
        }
        if self.waiting.len() > self.max_waiting as usize {
            return Err(InvalidPermits(format!(
                "at most maxWaiting ({}) requests wait, had {}",
                self.max_waiting,
                self.waiting.len()
            )));
        }
        let mut seen = HashSet::new();
        let held = self.holders.iter().map(|h| (&h.request, &h.message_type));
        for (request, message_type) in held.chain(self.waiting.iter().map(|w| (&w.request, &w.message_type))) {
            check_request(request)?;
            check_message_type(message_type)?;
            if !seen.insert(request.as_str()) {
                return Err(InvalidPermits(format!("the request '{request}' is in the books twice")));
            }
        }
        if let Some(holder) = self.holders.iter().find(|h| h.grant == 0 || h.grant >= self.next_grant) {
            return Err(InvalidPermits(format!(
                "a grant is at least 1 and below nextGrant ({}), was {}",
                self.next_grant, holder.grant
            )));
        }
        Ok(())
    }
}

fn check_capacity(capacity: u32) -> Result<(), InvalidPermits> {
    if capacity > MAX_SIZE {
        return Err(InvalidPermits(format!("capacity must be 0 through {MAX_SIZE}, was {capacity}")));
    }
    Ok(())
}

fn check_request(request: &str) -> Result<(), InvalidPermits> {
    let length = request.chars().count();
    if length == 0 || length > MAX_REQUEST_LENGTH || request.contains('\0') {
        return Err(InvalidPermits(format!(
            "a request ID must be 1 through {MAX_REQUEST_LENGTH} characters without NUL, had {length}"
        )));
    }
    Ok(())
}

fn check_message_type(message_type: &str) -> Result<(), InvalidPermits> {
    let length = message_type.chars().count();
    if length == 0 || length > MAX_TYPE_LENGTH || message_type.contains('\0') {
        return Err(InvalidPermits(format!(
            "messageType must be 1 through {MAX_TYPE_LENGTH} characters without NUL, had {length}"
        )));
    }
    Ok(())
}

/// The permit pool type. It has no settings: the capacity and the queue bound are in the state of each pool, given at
/// its create; a create of a pool that exists changes nothing. It never stops itself.
///
/// Each `Acquire` is answered in its own commit, with the crate's shared retry policy: `granted` with the next grant
/// number when fewer hold than the capacity and nobody waits, `queued` when the queue has room, `refused` otherwise. So
/// a target that is gone rejects the commit and nothing is recorded. After every message, while fewer hold than the
/// capacity and requests wait, the head of the queue becomes a holder with the next grant number, in the commit that
/// made room; its `granted` notice follows in a `Notify` that the pool sends itself, and so does each `check` of an
/// `Audit`. A `Notify` reads the state when it runs, so a holder that is gone fails only its own notice.
///
/// A delivered permit comes back only by a `Release` that names its grant number, from the holder or an operator; no
/// timeout and no audit frees one. A holder that is gone before its `granted` notice ever committed costs nothing: the
/// pool watches itself, and the `Failed` of its own grant `Notify` frees that permit and grants the next waiter. A
/// failed `check` frees nothing, and no `check` goes to a holder before its `granted` committed. Bad input is refused for good with the state unchanged: a capacity above 1,000, a
/// request ID or message type out of bounds, and a target or message type the send refuses (no registered type, or a
/// type that does not take the message); a read of the store that fails is tried again.
#[derive(Debug, Clone, Copy, Default)]
pub struct PermitsActor;

impl Actor for PermitsActor {
    type Ref = Permits;
    type State = PermitsState;

    async fn handle(&self, turn: &mut Turn<Self>, message: PermitsMsg) -> HandlerResult {
        match message {
            PermitsMsg::Acquire { request, to, message_type } => {
                check_request(&request).map_err(HandlerError::permanent)?;
                check_message_type(&message_type).map_err(HandlerError::permanent)?;
                acquire(turn, request, to, message_type).await?;
            }
            PermitsMsg::Release { request, grant } => {
                check_request(&request).map_err(HandlerError::permanent)?;
                if let Some(i) = turn.state().holders.iter().position(|h| h.request == request && h.grant == grant) {
                    turn.state_mut().holders.remove(i);
                }
            }
            PermitsMsg::Cancel { request } => {
                check_request(&request).map_err(HandlerError::permanent)?;
                if let Some(i) = turn.state().waiting.iter().position(|w| w.request == request) {
                    turn.state_mut().waiting.remove(i);
                }
            }
            PermitsMsg::Resize { capacity } => {
                check_capacity(capacity).map_err(HandlerError::permanent)?;
                if turn.state().capacity != capacity {
                    turn.state_mut().capacity = capacity;
                }
            }
            PermitsMsg::Audit => {
                let state = turn.state();
                let held = state.holders.iter().filter(|h| h.notified).map(|h| h.request.clone());
                let requests: Vec<String> = held.chain(state.waiting.iter().map(|w| w.request.clone())).collect();
                notify_later(turn, requests, true)?;
            }
            PermitsMsg::Notify { request, check } => {
                check_request(&request).map_err(HandlerError::permanent)?;
                if turn.message().from.as_ref() == Some(turn.self_address()) {
                    notify(turn, &request, check).await?;
                }
            }
            PermitsMsg::Failed(failed) if turn.message().notification => undelivered(turn, &failed),
            PermitsMsg::Failed(_) | PermitsMsg::Stopped(_) => {}
        }
        grant_waiting(turn)
    }
}

/// Answers an `Acquire` in this commit: again for a request in the books, otherwise by a grant, a place in the queue,
/// or a refusal that records nothing.
async fn acquire(
    turn: &mut Turn<PermitsActor>,
    request: String,
    to: ActorAddress,
    message_type: String,
) -> HandlerResult {
    let state = turn.state();
    if let Some(i) = state.holders.iter().position(|h| h.request == request) {
        let holder = &state.holders[i];
        let (to, message_type, grant, notified) =
            (holder.to.clone(), holder.message_type.clone(), holder.grant, holder.notified);
        tell(turn, &to, &message_type, &request, PermitNoticeKind::Granted, Some(grant)).await?;
        if !notified {
            turn.state_mut().holders[i].notified = true;
        }
        return Ok(());
    }
    if let Some(waiting) = state.waiting.iter().find(|w| w.request == request) {
        let (to, message_type) = (waiting.to.clone(), waiting.message_type.clone());
        return tell(turn, &to, &message_type, &request, PermitNoticeKind::Queued, None).await;
    }
    if state.holders.len() < state.capacity as usize && state.waiting.is_empty() {
        let grant = state.next_grant;
        let next_grant = next_grant(grant)?;
        tell(turn, &to, &message_type, &request, PermitNoticeKind::Granted, Some(grant)).await?;
        let state = turn.state_mut();
        state.next_grant = next_grant;
        state.holders.push(PermitHolder { request, to, message_type, grant, notified: true });
    } else if state.waiting.len() < state.max_waiting as usize {
        tell(turn, &to, &message_type, &request, PermitNoticeKind::Queued, None).await?;
        turn.state_mut().waiting.push(WaitingRequest { request, to, message_type });
    } else {
        tell(turn, &to, &message_type, &request, PermitNoticeKind::Refused, None).await?;
    }
    Ok(())
}

/// The grant number after `grant`; refused for good when the numbers are used up.
fn next_grant(grant: u64) -> Result<u64, HandlerError> {
    grant.checked_add(1).ok_or_else(|| HandlerError::permanent("the grant numbers of this pool are used up"))
}

/// Makes the head of the queue a holder while fewer hold than the capacity, and sends each its `granted` notice in a
/// turn of its own.
fn grant_waiting(turn: &mut Turn<PermitsActor>) -> HandlerResult {
    let mut granted = Vec::new();
    loop {
        let state = turn.state();
        if state.holders.len() >= state.capacity as usize || state.waiting.is_empty() {
            break;
        }
        let grant = state.next_grant;
        let next_grant = next_grant(grant)?;
        let state = turn.state_mut();
        let WaitingRequest { request, to, message_type } = state.waiting.remove(0);
        state.next_grant = next_grant;
        state.holders.push(PermitHolder { request: request.clone(), to, message_type, grant, notified: false });
        granted.push(request);
    }
    notify_later(turn, granted, false)
}

/// Sends the pool one `Notify` for each request, with the shared retry policy and this message's correlation id, and
/// watches the pool itself, so it hears of a `Notify` that fails.
fn notify_later(turn: &mut Turn<PermitsActor>, requests: Vec<String>, check: bool) -> HandlerResult {
    if requests.is_empty() {
        return Ok(());
    }
    for request in requests {
        let mut options = SendOptions::new().retry(retry_policy());
        options.correlation_id = turn.message().correlation_id.clone();
        turn.send_self_with(PermitsMsg::Notify { request, check }, options)?;
    }
    let itself = turn.self_address().clone();
    turn.watch(&itself)?;
    Ok(())
}

/// Sends `request` what is true now: a holder its `granted` notice, marked as notified, or its `check` with the grant
/// number once it was told of its grant; a waiting request its `check`; nothing otherwise. A waiter checked by an audit
/// may be granted before its `check` runs: it gets no `check` until its `granted` committed.
async fn notify(turn: &mut Turn<PermitsActor>, request: &str, check: bool) -> HandlerResult {
    let state = turn.state();
    if let Some(i) = state.holders.iter().position(|h| h.request == request) {
        let holder = &state.holders[i];
        let (to, message_type, grant, notified) =
            (holder.to.clone(), holder.message_type.clone(), holder.grant, holder.notified);
        if check && !notified {
            return Ok(());
        }
        let kind = if check { PermitNoticeKind::Check } else { PermitNoticeKind::Granted };
        tell(turn, &to, &message_type, request, kind, Some(grant)).await?;
        if !check && !notified {
            turn.state_mut().holders[i].notified = true;
        }
    } else if let Some(waiting) = state.waiting.iter().find(|w| w.request == request)
        && check
    {
        let (to, message_type) = (waiting.to.clone(), waiting.message_type.clone());
        tell(turn, &to, &message_type, request, PermitNoticeKind::Check, None).await?;
    }
    Ok(())
}

/// Sends a notice to the target of a request, with the shared retry policy and this message's correlation id. A send
/// the registry refuses is refused for good; a failed read of the store is tried again.
async fn tell(
    turn: &mut Turn<PermitsActor>,
    to: &ActorAddress,
    message_type: &str,
    request: &str,
    kind: PermitNoticeKind,
    grant: Option<u64>,
) -> HandlerResult {
    let notice = PermitNotice { pool: turn.self_ref().pool.clone(), request: request.to_owned(), kind, grant };
    let mut options = SendOptions::new().retry(retry_policy());
    options.correlation_id = turn.message().correlation_id.clone();
    match turn.send_raw(to, message_type, serde_json::to_value(notice)?, options).await {
        Ok(_) => Ok(()),
        // Tried again by the retry policy of the incoming message.
        Err(e @ TurnError::Store(_)) => Err(e.into()),
        // No registered type, a type that does not take the message, or a field out of bounds.
        Err(e) => Err(HandlerError::permanent_from(e)),
    }
}

/// Frees the permit of a grant `Notify` that the pool sent itself and that failed before the holder's `granted` notice
/// ever committed: the holder never learned its number, so no work runs under it.
fn undelivered(turn: &mut Turn<PermitsActor>, failed: &Failed) {
    let itself = turn.self_address();
    if &failed.actor != itself || failed.from.as_ref() != Some(itself) {
        return;
    }
    let Ok(PermitsMsg::Notify { request, check: false }) =
        cicada::message::decode::<PermitsMsg>(&failed.message_type, &failed.payload)
    else {
        return;
    };
    if let Some(i) = turn.state().holders.iter().position(|h| h.request == request && !h.notified) {
        turn.state_mut().holders.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holder(request: &str, grant: u64) -> serde_json::Value {
        serde_json::json!({"request": request, "to": "build/b", "messageType": "Permit", "grant": grant,
            "notified": true})
    }

    fn waiting(request: &str) -> serde_json::Value {
        serde_json::json!({"request": request, "to": "build/b", "messageType": "Permit"})
    }

    fn state(extra: serde_json::Value) -> serde_json::Value {
        let mut json = serde_json::json!({"capacity": 2, "maxWaiting": 2});
        json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json
    }

    #[test]
    fn the_constructor_takes_capacity_and_queue_bound_from_0_through_1000() {
        let state = PermitsState::new(2, 10).unwrap();
        assert_eq!((state.capacity, state.max_waiting, state.next_grant), (2, 10, 1));
        assert!(state.holders.is_empty() && state.waiting.is_empty());
        assert!(PermitsState::new(0, 0).is_ok(), "a paused pool without a queue");
        assert!(PermitsState::new(1000, 1000).is_ok());
        for (capacity, max_waiting, why) in [(1001, 0, "capacity must be"), (0, 1001, "maxWaiting must be")] {
            let built = PermitsState::new(capacity, max_waiting);
            assert!(built.as_ref().is_err_and(|e| e.to_string().contains(why)), "{built:?} for {why}");
        }
    }

    #[test]
    fn a_decoded_state_is_checked_like_a_built_one() {
        let decoded: PermitsState = serde_json::from_value(state(serde_json::json!({}))).unwrap();
        assert_eq!(decoded, PermitsState::new(2, 2).unwrap(), "nextGrant, holders and waiting may be left out");
        let full = state(serde_json::json!({"nextGrant": 5, "holders": [holder("a", 3), holder("b", 4)],
            "waiting": [waiting("c"), waiting("d")]}));
        assert!(serde_json::from_value::<PermitsState>(full).is_ok());
        let long = "r".repeat(MAX_REQUEST_LENGTH + 1);
        let long_type = "T".repeat(MAX_TYPE_LENGTH + 1);
        for (extra, why) in [
            (serde_json::json!({"capacity": 1001}), "capacity must be"),
            (serde_json::json!({"maxWaiting": 1001}), "maxWaiting must be"),
            (serde_json::json!({"nextGrant": 0}), "nextGrant must be at least 1"),
            (serde_json::json!({"waiting": [waiting("a"), waiting("b"), waiting("c")]}), "at most maxWaiting"),
            (serde_json::json!({"nextGrant": 5, "holders": [holder("a", 3), holder("a", 4)]}), "twice"),
            (serde_json::json!({"nextGrant": 5, "holders": [holder("a", 3)], "waiting": [waiting("a")]}), "twice"),
            (serde_json::json!({"waiting": [waiting("a"), waiting("a")]}), "twice"),
            (serde_json::json!({"nextGrant": 3, "holders": [holder("a", 3)]}), "below nextGrant"),
            (serde_json::json!({"nextGrant": 3, "holders": [holder("a", 0)]}), "below nextGrant"),
            (serde_json::json!({"waiting": [waiting("")]}), "a request ID must be"),
            (serde_json::json!({"waiting": [waiting(&long)]}), "a request ID must be"),
            (serde_json::json!({"nextGrant": 2, "holders": [holder("a\0b", 1)]}), "a request ID must be"),
            (
                serde_json::json!({"waiting": [{"request": "a", "to": "build/b", "messageType": long_type}]}),
                "messageType must be",
            ),
        ] {
            let json = state(extra);
            let decoded = serde_json::from_value::<PermitsState>(json.clone());
            assert!(decoded.as_ref().is_err_and(|e| e.to_string().contains(why)), "{decoded:?} for {json}");
        }
        for missing in ["capacity", "maxWaiting"] {
            let mut json = state(serde_json::json!({}));
            json.as_object_mut().unwrap().remove(missing);
            assert!(serde_json::from_value::<PermitsState>(json).is_err(), "{missing} is required");
        }
    }

    #[test]
    fn more_holders_than_the_capacity_is_a_state_after_a_shrink() {
        let json = serde_json::json!({"capacity": 1, "maxWaiting": 0, "nextGrant": 3,
            "holders": [holder("a", 1), holder("b", 2)]});
        assert!(serde_json::from_value::<PermitsState>(json).is_ok());
    }

    #[test]
    fn the_acquire_helper_checks_the_request_and_the_message_type() {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct Build;
        impl ActorRef for Build {
            const TYPE: &'static str = "build";
            type Message = PermitsMsg;
        }
        assert!(PermitsMsg::acquire("r".repeat(MAX_REQUEST_LENGTH), &Build, "Permit").is_ok());
        for (request, message_type) in [
            ("".to_owned(), "Permit".to_owned()),
            ("r".repeat(MAX_REQUEST_LENGTH + 1), "Permit".to_owned()),
            ("a\0".to_owned(), "Permit".to_owned()),
            ("a".to_owned(), "".to_owned()),
            ("a".to_owned(), "T".repeat(MAX_TYPE_LENGTH + 1)),
        ] {
            assert!(PermitsMsg::acquire(&request, &Build, &message_type).is_err(), "{request:?} {message_type:?}");
        }
    }

    #[test]
    fn grant_numbers_end_at_the_largest_number() {
        assert_eq!(next_grant(1).unwrap(), 2);
        assert!(next_grant(u64::MAX).is_err());
    }
}
