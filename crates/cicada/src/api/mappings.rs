//! Conversions between the runtime model and the HTTP bodies.

use super::{
    ActorView, BulkResponse, MailboxMessageView, MessageState, MessageStatusView, MessageView, NodeDetailView,
    NodeView, OutcomeView, PageView, RequestError, RunningTurnView, SendRequest, StatsView, TypeView, WatchView,
};
use crate::runtime::{ActorSnapshot, BulkResult, MailboxEntry, NodeInfo, RegisteredType};
use crate::store::{Delivery, Message, MessageStatus, OutcomeRecord, OutcomeStatus, Page, RunningTurn, StoreStats};
use crate::watch::Watch;

impl From<Message> for MessageView {
    fn from(m: Message) -> Self {
        Self {
            id: m.id,
            to: m.to,
            from: m.from,
            message_type: m.message_type,
            payload: m.payload,
            deliver_at: m.deliver_at,
            correlation_id: m.correlation_id,
            headers: m.headers,
            seq: m.seq,
            attempt: m.attempt,
            failures: m.failures,
            lost_claims: m.lost_claims,
            last_error: m.last_error,
            retry: m.retry,
            caused_by: m.caused_by,
            retention: m.retention,
            notification: m.notification,
        }
    }
}

impl From<Watch> for WatchView {
    fn from(w: Watch) -> Self {
        Self { target: w.target, watcher: w.watcher }
    }
}

impl From<OutcomeStatus> for MessageState {
    fn from(status: OutcomeStatus) -> Self {
        match status {
            OutcomeStatus::Done => MessageState::Done,
            OutcomeStatus::Failed => MessageState::Failed,
            OutcomeStatus::Cancelled => MessageState::Cancelled,
            OutcomeStatus::Purged => MessageState::Purged,
        }
    }
}

impl From<OutcomeRecord> for OutcomeView {
    fn from(o: OutcomeRecord) -> Self {
        Self {
            id: o.message_id,
            actor: o.actor,
            status: o.status.into(),
            error: o.error,
            finished_at: o.finished_at,
            message: o.envelope.into(),
        }
    }
}

impl From<MessageStatus> for MessageStatusView {
    fn from(status: MessageStatus) -> Self {
        match status {
            MessageStatus::Pending(m) => Self {
                id: m.id.clone(),
                status: MessageState::Pending,
                error: None,
                failures: m.failures,
                last_error: m.last_error.clone(),
                finished_at: None,
                node: None,
                lease_until: None,
                message: m.into(),
            },
            MessageStatus::Running { message: m, node, lease_until } => Self {
                id: m.id.clone(),
                status: MessageState::Running,
                error: None,
                failures: m.failures,
                last_error: m.last_error.clone(),
                finished_at: None,
                node: Some(node.to_string()),
                lease_until: Some(lease_until),
                message: m.into(),
            },
            MessageStatus::Finished(o) => Self {
                id: o.message_id,
                status: o.status.into(),
                error: o.error,
                failures: o.envelope.failures,
                last_error: o.envelope.last_error.clone(),
                finished_at: Some(o.finished_at),
                node: None,
                lease_until: None,
                message: o.envelope.into(),
            },
        }
    }
}

impl From<StoreStats> for StatsView {
    fn from(s: StoreStats) -> Self {
        Self { actors: s.actors, pending: s.pending, oldest_due: s.oldest_due, failed: Some(s.failed) }
    }
}

impl From<ActorSnapshot> for ActorView {
    fn from(s: ActorSnapshot) -> Self {
        Self {
            type_name: s.address.type_name().as_str().to_owned(),
            address: s.address,
            state: s.state,
            version: s.version,
            created_at: s.created_at,
            paused: s.paused,
            parent: s.parent,
            pinned_to: s.pinned_to,
        }
    }
}

impl From<RunningTurn> for RunningTurnView {
    fn from(t: RunningTurn) -> Self {
        Self {
            actor: t.actor,
            message_id: t.message_id,
            node: t.node,
            lease_until: t.lease_until,
            cancel_requested: t.cancel_requested,
        }
    }
}

impl From<&NodeInfo> for NodeView {
    fn from(n: &NodeInfo) -> Self {
        Self {
            id: n.id.clone(),
            hosted: n.hosted.iter().map(|t| t.as_str().to_owned()).collect(),
            started_at: n.started_at,
            last_seen: n.last_seen,
            status: n.status,
            running_turns: n.running_turns.len(),
            hostname: n.hostname.clone(),
            cicada_version: n.cicada_version.clone(),
            label: n.label.clone(),
        }
    }
}

impl From<NodeInfo> for NodeDetailView {
    fn from(n: NodeInfo) -> Self {
        Self { node: NodeView::from(&n), turns: n.running_turns.into_iter().map(RunningTurnView::from).collect() }
    }
}

impl From<MailboxEntry> for MailboxMessageView {
    fn from(e: MailboxEntry) -> Self {
        Self { message: e.message.into(), state: if e.running { MessageState::Running } else { MessageState::Pending } }
    }
}

impl From<BulkResult> for BulkResponse {
    fn from(r: BulkResult) -> Self {
        Self { affected: r.affected, more: r.more }
    }
}

impl From<&RegisteredType> for TypeView {
    fn from(t: &RegisteredType) -> Self {
        Self {
            name: t.name().as_str().to_owned(),
            hash: t.definition.contract_hash.clone(),
            state: t.contract.state_schema().clone(),
            messages: t.contract.message_schemas().clone(),
        }
    }
}

impl<T> PageView<T> {
    /// The wire form of `page`, converting each item with `f`.
    pub fn of<S>(page: Page<S>, f: impl FnMut(S) -> T) -> Self {
        Self { items: page.items.into_iter().map(f).collect(), next: page.next.map(|c| c.0) }
    }
}

impl SendRequest {
    /// The delivery time this send asks for: [`Self::deliver_at`], [`Self::delay`] or now. Fails when both are given;
    /// the range of the time is checked by the send itself.
    pub fn delivery(&self) -> Result<Delivery, RequestError> {
        match (self.deliver_at, self.delay) {
            (Some(_), Some(_)) => Err(RequestError("Give either deliverAt or delay, not both".to_owned())),
            (Some(at), None) => Ok(Delivery::At(at)),
            (None, Some(after)) => Ok(Delivery::After(after)),
            (None, None) => Ok(Delivery::Now),
        }
    }

    /// Sets [`Self::deliver_at`] or [`Self::delay`] from `delivery`.
    pub fn with_delivery(mut self, delivery: Delivery) -> Self {
        (self.deliver_at, self.delay) = match delivery {
            Delivery::Now => (None, None),
            Delivery::At(at) => (Some(at), None),
            Delivery::After(after) => (None, Some(after)),
        };
        self
    }
}
