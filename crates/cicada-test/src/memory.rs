//! A complete store held in memory.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cicada::address::{ActorAddress, UriTemplate};
use cicada::store::support::{self, check_limit};
use cicada::store::{
    ActorRecord, CancelResult, Claim, Clock, CommitRequest, CommitResult, CreateResult, DeleteResult, EnqueueResult,
    Message, MessageId, MessageStatus, NodeId, OutcomeRecord, OutcomeStatus, OutgoingMessage, Page, PageCursor,
    RegisterResult, RenewResult, StateChange, Store, StoreError, StoreResult, SystemClock, TurnOutcome, TypeDef,
};
use cicada::time::{self, DEFAULT_RETENTION, Timestamp};
use serde_json::Value;

struct ActorRow {
    address: ActorAddress,
    template: UriTemplate,
    state: Option<Value>,
    version: i64,
    created_at: Timestamp,
    epoch: i64,
    lease_owner: Option<NodeId>,
    lease_until: Option<Timestamp>,
    lease_message: Option<MessageId>,
    /// Pending messages ordered by `(deliver_at, seq)`.
    mailbox: BTreeSet<(Timestamp, i64, MessageId)>,
}

impl ActorRow {
    fn has_valid_lease(&self, now: Timestamp) -> bool {
        self.lease_owner.is_some() && self.lease_until.is_some_and(|u| u > now)
    }

    fn clear_lease(&mut self) {
        self.lease_owner = None;
        self.lease_until = None;
        self.lease_message = None;
    }

    fn snapshot(&self) -> ActorRecord {
        ActorRecord {
            address: self.address.clone(),
            template: self.template.clone(),
            state: self.state.clone(),
            version: self.version,
            created_at: self.created_at,
        }
    }

    fn head(&self) -> Option<&(Timestamp, i64, MessageId)> {
        self.mailbox.iter().next()
    }
}

/// A retained outcome with its retention: visible until `visible_until` (`None`: for the store retention), and part
/// of the trimming of its instance, in `keep_seq` order, when `keep_seq` is set.
struct OutcomeRow {
    record: OutcomeRecord,
    visible_until: Option<Timestamp>,
    keep_seq: Option<i64>,
    stripped: bool,
}

#[derive(Default)]
struct State {
    types: BTreeMap<String, TypeDef>,
    actors: BTreeMap<String, ActorRow>,
    messages: HashMap<MessageId, Message>,
    outcomes: HashMap<MessageId, OutcomeRow>,
    last_seq: i64,
    last_epoch: i64,
    last_now: Option<Timestamp>,
}

/// A complete [`Store`] held in memory, for tests and single-process experiments. Passes the S1–S10 conformance suite
/// except S1 (durability across process restarts), which an in-memory store cannot offer.
///
/// Every operation runs under one lock and never awaits while holding it, so all operations are linearizable (S3) and
/// commits are atomic (S2). Store time never goes backwards: when the clock steps back, the store keeps the latest
/// time it handed out until the clock catches up.
pub struct InMemoryStore {
    clock: Arc<dyn Clock>,
    retention: Duration,
    state: Mutex<State>,
    closed: AtomicBool,
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new(Arc::new(SystemClock), DEFAULT_RETENTION)
    }
}

impl InMemoryStore {
    /// A store on `clock` that keeps outcomes for `retention`.
    pub fn new(clock: Arc<dyn Clock>, retention: Duration) -> Self {
        Self { clock, retention, state: Mutex::new(State::default()), closed: AtomicBool::new(false) }
    }

    /// A store on `clock` with the default retention.
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self::new(clock, DEFAULT_RETENTION)
    }

    fn locked<T>(&self, f: impl FnOnce(&mut State, Timestamp) -> StoreResult<T>) -> StoreResult<T> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(StoreError::unavailable("Store is closed"));
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut now = time::truncate(self.clock.now());
        if let Some(last) = state.last_now
            && last > now
        {
            now = last;
        }
        state.last_now = Some(now);
        f(&mut state, now)
    }

    fn cutoff(&self, now: Timestamp) -> Timestamp {
        time::sub(now, self.retention)
    }

    fn is_visible(&self, row: &OutcomeRow, now: Timestamp) -> bool {
        match row.visible_until {
            Some(until) => until > now,
            None => row.record.finished_at > self.cutoff(now),
        }
    }

    /// Still within the dedup window (or kept visible longer than it).
    fn is_retained(&self, row: &OutcomeRow, now: Timestamp) -> bool {
        row.record.finished_at > self.cutoff(now) || self.is_visible(row, now)
    }

    fn is_id_in_use(&self, s: &State, id: &MessageId, now: Timestamp) -> bool {
        s.messages.contains_key(id) || s.outcomes.get(id).is_some_and(|r| self.is_retained(r, now))
    }

    fn live_outcome(&self, s: &State, id: &MessageId, now: Timestamp) -> Option<OutcomeRecord> {
        s.outcomes.get(id).filter(|r| self.is_visible(r, now)).map(|r| r.record.clone())
    }

    fn insert(&self, s: &mut State, message: OutgoingMessage, now: Timestamp) -> StoreResult<Message> {
        let deliver_at = message.delivery.resolve(now).map_err(|e| StoreError::Invalid(e.0))?;
        s.outcomes.remove(&message.id); // an expired, not yet swept outcome with the same id
        s.last_seq += 1;
        let stored = message.into_message(deliver_at, s.last_seq);
        let row = s.actors.get_mut(stored.to.as_str()).expect("target exists");
        row.mailbox.insert((stored.deliver_at, stored.seq, stored.id.clone()));
        s.messages.insert(stored.id.clone(), stored.clone());
        Ok(stored)
    }

    fn finish(
        &self,
        s: &mut State,
        message: Message,
        status: OutcomeStatus,
        error: Option<String>,
        now: Timestamp,
    ) -> OutcomeRecord {
        s.messages.remove(&message.id);
        let record = OutcomeRecord {
            message_id: message.id.clone(),
            actor: message.to.clone(),
            status,
            error: error.clone(),
            finished_at: now,
            envelope: message.clone(),
        };
        let kept = if support::removed_at_once(&message, status) {
            OutcomeRecord { error: None, envelope: support::stripped(&message), ..record.clone() }
        } else {
            record.clone()
        };
        let keep_last = support::keep_last(&message, status);
        let keep_seq = keep_last.map(|_| {
            s.last_seq += 1;
            s.last_seq
        });
        s.outcomes.insert(
            message.id.clone(),
            OutcomeRow {
                record: kept,
                visible_until: support::visible_until(&message, status, now),
                keep_seq,
                stripped: false,
            },
        );
        if let Some(count) = keep_last {
            self.trim(s, &message.to, status, count as usize, now);
        }
        record
    }

    /// Hides the visible keep-last outcomes of `actor` with `status` beyond the newest `count`.
    fn trim(&self, s: &mut State, actor: &ActorAddress, status: OutcomeStatus, count: usize, now: Timestamp) {
        let mut rows: Vec<(i64, MessageId)> = s
            .outcomes
            .iter()
            .filter(|(_, r)| {
                r.keep_seq.is_some() && &r.record.actor == actor && r.record.status == status && self.is_visible(r, now)
            })
            .map(|(id, r)| (r.keep_seq.unwrap_or_default(), id.clone()))
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        for (_, id) in rows.into_iter().skip(count) {
            if let Some(r) = s.outcomes.get_mut(&id) {
                r.visible_until = Some(now);
            }
        }
    }

    fn purge_mailbox(&self, s: &mut State, address: &ActorAddress, now: Timestamp) -> usize {
        let ids: Vec<MessageId> = match s.actors.get_mut(address.as_str()) {
            Some(row) => {
                row.clear_lease();
                std::mem::take(&mut row.mailbox).into_iter().map(|(_, _, id)| id).collect()
            }
            None => return 0,
        };
        for id in &ids {
            if let Some(m) = s.messages.get(id).cloned() {
                self.finish(s, m, OutcomeStatus::Purged, None, now);
            }
        }
        ids.len()
    }

    fn status_locked(&self, s: &State, id: &MessageId, now: Timestamp) -> Option<MessageStatus> {
        if let Some(message) = s.messages.get(id) {
            let row = s.actors.get(message.to.as_str())?;
            return Some(if row.lease_message.as_ref() == Some(id) && row.has_valid_lease(now) {
                MessageStatus::Running {
                    message: message.clone(),
                    node: row.lease_owner.clone().expect("valid lease has an owner"),
                    lease_until: row.lease_until.expect("valid lease has an expiry"),
                }
            } else {
                MessageStatus::Pending(message.clone())
            });
        }
        self.live_outcome(s, id, now).map(MessageStatus::Finished)
    }

    fn create_locked(
        s: &mut State,
        address: &ActorAddress,
        template: &UriTemplate,
        state: Option<Value>,
        now: Timestamp,
    ) -> CreateResult {
        if let Some(row) = s.actors.get(address.as_str()) {
            return CreateResult::Exists(row.snapshot());
        }
        s.actors.insert(
            address.as_str().to_owned(),
            ActorRow {
                address: address.clone(),
                template: template.clone(),
                state,
                version: 0,
                created_at: now,
                epoch: 0,
                lease_owner: None,
                lease_until: None,
                lease_message: None,
                mailbox: BTreeSet::new(),
            },
        );
        CreateResult::Created
    }
}

fn held_lease<'a>(s: &'a mut State, address: &ActorAddress, epoch: i64) -> Option<&'a mut ActorRow> {
    s.actors.get_mut(address.as_str()).filter(|r| r.epoch == epoch && r.lease_owner.is_some())
}

#[async_trait]
impl Store for InMemoryStore {
    async fn now(&self) -> StoreResult<Timestamp> {
        self.locked(|_, now| Ok(now))
    }

    async fn register(&self, def: TypeDef) -> StoreResult<RegisterResult> {
        self.locked(|s, _| {
            if let Some(existing) = s.types.get(def.template.as_str()) {
                return Ok(support::register_again(existing.clone(), &def));
            }
            let registered: Vec<TypeDef> = s.types.values().cloned().collect();
            if let Some(overlapping) = support::overlapping(&registered, &def.template) {
                return Ok(RegisterResult::Conflict {
                    existing: overlapping.clone(),
                    reason: support::overlap_reason(&def.template, &overlapping.template),
                });
            }
            s.types.insert(def.template.as_str().to_owned(), def.clone());
            Ok(RegisterResult::Registered(def))
        })
    }

    async fn list_types(&self) -> StoreResult<Vec<TypeDef>> {
        self.locked(|s, _| Ok(s.types.values().cloned().collect()))
    }

    async fn create(
        &self,
        address: &ActorAddress,
        template: &UriTemplate,
        state: Option<Value>,
    ) -> StoreResult<CreateResult> {
        self.locked(|s, now| Ok(Self::create_locked(s, address, template, state, now)))
    }

    async fn get(&self, address: &ActorAddress) -> StoreResult<Option<ActorRecord>> {
        self.locked(|s, _| Ok(s.actors.get(address.as_str()).map(ActorRow::snapshot)))
    }

    async fn list(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize) -> StoreResult<Page<ActorAddress>> {
        check_limit(limit)?;
        self.locked(|s, _| {
            use std::ops::Bound;
            let start = match cursor {
                Some(c) => Bound::Excluded(c.0.clone()),
                None => Bound::Included(prefix.to_owned()),
            };
            let items: Vec<ActorAddress> = s
                .actors
                .range((start, Bound::Unbounded))
                .map(|(_, r)| &r.address)
                .skip_while(|a| !a.as_str().starts_with(prefix) && a.as_str() < prefix)
                .take_while(|a| a.as_str().starts_with(prefix))
                .take(limit + 1)
                .cloned()
                .collect();
            Ok(support::page_of(items, limit, |a| PageCursor(a.to_string())))
        })
    }

    async fn delete(&self, address: &ActorAddress) -> StoreResult<DeleteResult> {
        self.locked(|s, now| {
            if !s.actors.contains_key(address.as_str()) {
                return Ok(DeleteResult::Absent);
            }
            let purged = self.purge_mailbox(s, address, now);
            s.actors.remove(address.as_str());
            Ok(DeleteResult::Deleted { purged })
        })
    }

    async fn enqueue(&self, message: OutgoingMessage) -> StoreResult<EnqueueResult> {
        self.locked(|s, now| {
            if self.is_id_in_use(s, &message.id, now) {
                return Ok(EnqueueResult::Duplicate(message.id));
            }
            if !s.actors.contains_key(message.to.as_str()) {
                return Ok(EnqueueResult::NoSuchInstance);
            }
            Ok(EnqueueResult::Enqueued(self.insert(s, message, now)?))
        })
    }

    async fn claim_due(
        &self,
        types: &BTreeSet<UriTemplate>,
        node: &NodeId,
        ttl: Duration,
        limit: usize,
    ) -> StoreResult<Vec<Claim>> {
        if !support::check_claim(types, ttl, limit)? {
            return Ok(Vec::new());
        }
        self.locked(|s, now| {
            let mut due: Vec<(Timestamp, String)> = s
                .actors
                .values()
                .filter(|r| types.contains(&r.template) && !r.has_valid_lease(now))
                .filter_map(|r| r.head().filter(|h| h.0 <= now).map(|h| (h.0, r.address.as_str().to_owned())))
                .collect();
            due.sort();
            due.truncate(limit);
            let lease_until = time::truncate(time::add(now, ttl).map_err(|e| StoreError::Invalid(e.0))?);
            let mut claims = Vec::with_capacity(due.len());
            for (_, uri) in due {
                s.last_epoch += 1;
                let epoch = s.last_epoch;
                let row = s.actors.get_mut(&uri).expect("candidate exists");
                let head_id = row.head().expect("due head").2.clone();
                let message = s.messages.get_mut(&head_id).expect("head message exists");
                message.attempt += 1;
                let claimed = message.clone();
                row.epoch = epoch;
                row.lease_owner = Some(node.clone());
                row.lease_until = Some(lease_until);
                row.lease_message = Some(claimed.id.clone());
                claims.push(Claim {
                    actor: row.address.clone(),
                    template: row.template.clone(),
                    epoch,
                    message: claimed,
                    state: row.state.clone(),
                    version: row.version,
                    claimed_at: now,
                    lease_until,
                });
            }
            Ok(claims)
        })
    }

    async fn renew(&self, address: &ActorAddress, epoch: i64, ttl: Duration) -> StoreResult<RenewResult> {
        self.locked(|s, now| {
            let lease_until = time::truncate(time::add(now, ttl).map_err(|e| StoreError::Invalid(e.0))?);
            Ok(match held_lease(s, address, epoch) {
                Some(row) => {
                    row.lease_until = Some(lease_until);
                    RenewResult::Renewed(lease_until)
                }
                None => RenewResult::LeaseLost,
            })
        })
    }

    async fn release(&self, address: &ActorAddress, epoch: i64) -> StoreResult<()> {
        self.locked(|s, _| {
            if let Some(row) = held_lease(s, address, epoch) {
                row.clear_lease();
            }
            Ok(())
        })
    }

    async fn next_due_at(&self, types: &BTreeSet<UriTemplate>) -> StoreResult<Option<Timestamp>> {
        self.locked(|s, now| {
            Ok(s.actors
                .values()
                .filter(|r| types.contains(&r.template))
                .filter_map(|r| {
                    let head = r.head()?.0;
                    Some(if r.has_valid_lease(now) { head.max(r.lease_until.expect("valid lease")) } else { head })
                })
                .min())
        })
    }

    async fn pending(
        &self,
        address: &ActorAddress,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_mailbox_cursor).transpose()?;
        self.locked(|s, _| {
            let Some(row) = s.actors.get(address.as_str()) else {
                return Ok(Page::new(Vec::new(), None));
            };
            let items: Vec<Message> = row
                .mailbox
                .iter()
                .filter(|(at, seq, _)| match after {
                    None => true,
                    Some((a, q)) => {
                        let at = time::to_micros(*at);
                        at > a || (at == a && *seq > q)
                    }
                })
                .take(limit + 1)
                .map(|(_, _, id)| s.messages[id].clone())
                .collect();
            Ok(support::page_of(items, limit, support::mailbox_cursor))
        })
    }

    async fn list_pending(
        &self,
        prefix: &str,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<Message>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_pending_cursor).transpose()?;
        self.locked(|s, _| {
            let from = match &after {
                Some((uri, _, _)) if uri.as_str() >= prefix => uri.clone(),
                _ => prefix.to_owned(),
            };
            let items: Vec<Message> = s
                .actors
                .range(from..)
                .map(|(_, r)| r)
                .take_while(|r| r.address.as_str().starts_with(prefix))
                .flat_map(|r| r.mailbox.iter().map(|(_, _, id)| &s.messages[id]))
                .filter(|m| match &after {
                    None => true,
                    Some((uri, at, seq)) => {
                        let m_at = time::to_micros(m.deliver_at);
                        m.to.as_str() > uri.as_str()
                            || (m.to.as_str() == uri && (m_at > *at || (m_at == *at && m.seq > *seq)))
                    }
                })
                .take(limit + 1)
                .cloned()
                .collect();
            Ok(support::page_of(items, limit, support::pending_cursor))
        })
    }

    async fn cancel(&self, id: &MessageId) -> StoreResult<CancelResult> {
        self.locked(|s, now| {
            let Some(message) = s.messages.get(id).cloned() else {
                return Ok(match self.live_outcome(s, id, now) {
                    Some(o) => CancelResult::AlreadyFinished(o),
                    None => CancelResult::NotFound,
                });
            };
            let row = s.actors.get_mut(message.to.as_str()).expect("target exists");
            if row.lease_message.as_ref() == Some(id) && row.lease_owner.is_some() {
                if row.has_valid_lease(now) {
                    return Ok(CancelResult::Running);
                }
                row.clear_lease(); // void the expired lease so the stale turn cannot commit
            }
            row.mailbox.remove(&(message.deliver_at, message.seq, message.id.clone()));
            Ok(CancelResult::Cancelled(self.finish(s, message, OutcomeStatus::Cancelled, None, now)))
        })
    }

    async fn status(&self, id: &MessageId) -> StoreResult<Option<MessageStatus>> {
        self.locked(|s, now| Ok(self.status_locked(s, id, now)))
    }

    async fn failures(
        &self,
        prefix: &str,
        since: Timestamp,
        cursor: Option<&PageCursor>,
        limit: usize,
    ) -> StoreResult<Page<OutcomeRecord>> {
        check_limit(limit)?;
        let after = cursor.map(support::decode_failure_cursor).transpose()?;
        self.locked(|s, now| {
            let mut items: Vec<OutcomeRecord> = s
                .outcomes
                .values()
                .filter(|r| self.is_visible(r, now))
                .map(|r| &r.record)
                .filter(|r| r.status == OutcomeStatus::Failed && r.actor.has_prefix(prefix) && r.finished_at >= since)
                .filter(|r| match &after {
                    None => true,
                    Some((at, id)) => {
                        let f = time::to_micros(r.finished_at);
                        f > *at || (f == *at && r.message_id.as_str() > id.as_str())
                    }
                })
                .cloned()
                .collect();
            items.sort_by(|a, b| (a.finished_at, a.message_id.as_str()).cmp(&(b.finished_at, b.message_id.as_str())));
            items.truncate(limit + 1);
            Ok(support::page_of(items, limit, |o| support::failure_cursor(o.finished_at, &o.message_id)))
        })
    }

    async fn sweep_expired(&self) -> StoreResult<u64> {
        self.locked(|s, now| {
            // Hidden records lose their content; records past both the dedup window and their visibility go.
            let hidden: Vec<MessageId> = s
                .outcomes
                .iter()
                .filter(|(_, r)| !r.stripped && !self.is_visible(r, now))
                .map(|(id, _)| id.clone())
                .collect();
            for id in hidden {
                if let Some(r) = s.outcomes.get_mut(&id) {
                    r.record.error = None;
                    r.record.envelope = support::stripped(&r.record.envelope);
                    r.stripped = true;
                }
            }
            let expired: Vec<MessageId> =
                s.outcomes.iter().filter(|(_, r)| !self.is_retained(r, now)).map(|(id, _)| id.clone()).collect();
            for id in &expired {
                s.outcomes.remove(id);
            }
            Ok(expired.len() as u64)
        })
    }

    async fn commit(&self, commit: CommitRequest) -> StoreResult<CommitResult> {
        self.locked(|s, now| {
            let Some(row) = held_lease(s, &commit.address, commit.epoch) else {
                return Ok(CommitResult::LeaseLost);
            };
            if row.lease_message.as_ref() != Some(&commit.message_id) {
                return Err(StoreError::Invalid(format!(
                    "Message {} is not the message claimed under epoch {}",
                    commit.message_id, commit.epoch
                )));
            }
            if commit.cancels.contains(&commit.message_id) {
                return Err(StoreError::Invalid(format!("A turn cannot cancel its own message {}", commit.message_id)));
            }
            // Validate everything before changing anything (S2).
            let mut send_ids = HashSet::new();
            for send in &commit.sends {
                let exists =
                    s.actors.contains_key(send.to.as_str()) || commit.creates.iter().any(|c| c.address == send.to);
                if !exists {
                    return Ok(CommitResult::Rejected(format!("Send to absent instance {}", send.to)));
                }
                if !send_ids.insert(send.id.clone())
                    || send.id == commit.message_id
                    || self.is_id_in_use(s, &send.id, now)
                {
                    return Ok(CommitResult::Rejected(format!("Message id {} is already in use", send.id)));
                }
                send.delivery.resolve(now).map_err(|e| StoreError::Invalid(e.0))?;
            }
            let reschedule_at = match &commit.outcome {
                TurnOutcome::Rescheduled { delivery, .. } => {
                    Some(delivery.resolve(now).map_err(|e| StoreError::Invalid(e.0))?)
                }
                _ => None,
            };

            // 1. outcome, or back to pending
            let message = s.messages.get(&commit.message_id).cloned().expect("claimed message is pending");
            let row = s.actors.get_mut(commit.address.as_str()).expect("lease holder exists");
            row.mailbox.remove(&(message.deliver_at, message.seq, message.id.clone()));
            match &commit.outcome {
                TurnOutcome::Rescheduled { error, .. } => {
                    s.last_seq += 1;
                    let again = Message {
                        deliver_at: reschedule_at.expect("resolved above"),
                        seq: s.last_seq,
                        failures: message.failures + 1,
                        last_error: Some(error.clone()),
                        ..message
                    };
                    row.mailbox.insert((again.deliver_at, again.seq, again.id.clone()));
                    s.messages.insert(again.id.clone(), again);
                }
                outcome => {
                    let status = outcome.status().expect("final outcome");
                    self.finish(s, message, status, outcome.error().map(str::to_owned), now);
                }
            }
            // 2. cancels: only this instance's pending messages
            let mut cancelled = Vec::new();
            let mut seen = HashSet::new();
            for id in commit.cancels.iter().filter(|id| seen.insert(*id)) {
                let Some(m) = s.messages.get(id).filter(|m| m.to == commit.address).cloned() else {
                    continue;
                };
                if let Some(row) = s.actors.get_mut(commit.address.as_str()) {
                    row.mailbox.remove(&(m.deliver_at, m.seq, m.id.clone()));
                }
                self.finish(s, m, OutcomeStatus::Cancelled, None, now);
                cancelled.push(id.clone());
            }
            // 3. creates, 4. sends
            for c in &commit.creates {
                Self::create_locked(s, &c.address, &c.template, c.state.clone(), now);
            }
            let mut sent = Vec::with_capacity(commit.sends.len());
            for send in commit.sends {
                sent.push(self.insert(s, send, now)?);
            }
            // 5. state, 6. release
            let version = match commit.state {
                StateChange::Unchanged => s.actors[commit.address.as_str()].version,
                StateChange::NewState(state) => {
                    let row = s.actors.get_mut(commit.address.as_str()).expect("lease holder exists");
                    row.state = state;
                    row.version += 1;
                    row.version
                }
                StateChange::Stop => {
                    let version = s.actors[commit.address.as_str()].version;
                    self.purge_mailbox(s, &commit.address, now);
                    s.actors.remove(commit.address.as_str());
                    version
                }
            };
            if let Some(row) = s.actors.get_mut(commit.address.as_str()) {
                row.clear_lease();
            }
            Ok(CommitResult::Committed { version, sent, cancelled })
        })
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}
