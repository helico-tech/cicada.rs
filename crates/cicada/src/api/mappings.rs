//! Conversions between the runtime model and the HTTP bodies.

use super::{
    ActorView, ChainNodeView, MessageChainView, MessageState, MessageStatusView, MessageView, OutcomeView, PageView,
    SendRequest, TypeMatchView, TypeView,
};
use crate::runtime::{ActorSnapshot, MessageChain, RegisteredType, ResolvedActorType};
use crate::store::{ChainEntry, Delivery, Message, MessageStatus, OutcomeRecord, OutcomeStatus, Page};

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
            last_error: m.last_error,
            retry: m.retry,
            caused_by: m.caused_by,
            retention: m.retention,
        }
    }
}

impl From<MessageView> for Message {
    fn from(v: MessageView) -> Self {
        Self {
            id: v.id,
            to: v.to,
            from: v.from,
            message_type: v.message_type,
            payload: v.payload,
            deliver_at: v.deliver_at,
            correlation_id: v.correlation_id,
            headers: v.headers,
            seq: v.seq,
            attempt: v.attempt,
            failures: v.failures,
            last_error: v.last_error,
            retry: v.retry,
            caused_by: v.caused_by,
            retention: v.retention,
        }
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

impl From<ChainEntry> for ChainNodeView {
    fn from(entry: ChainEntry) -> Self {
        match entry {
            ChainEntry::Live(status) => {
                let finished = matches!(status, MessageStatus::Finished(_));
                let view = MessageStatusView::from(status);
                let m = view.message;
                Self {
                    id: m.id,
                    caused_by: m.caused_by,
                    to: m.to,
                    from: m.from,
                    message_type: m.message_type,
                    status: view.status,
                    deliver_at: m.deliver_at,
                    finished_at: view.finished_at,
                    // A finished message's own error; a waiting one's last retried error.
                    error: if finished { view.error } else { m.last_error },
                    attempt: m.attempt,
                    failures: m.failures,
                    removed: false,
                }
            }
            ChainEntry::Removed { status, finished_at, envelope } => Self {
                id: envelope.id,
                caused_by: envelope.caused_by,
                to: envelope.to,
                from: envelope.from,
                message_type: envelope.message_type,
                status: status.into(),
                deliver_at: envelope.deliver_at,
                finished_at: Some(finished_at),
                error: None,
                attempt: envelope.attempt,
                failures: envelope.failures,
                removed: true,
            },
        }
    }
}

impl From<MessageChain> for MessageChainView {
    /// The wire form of the chain, its nodes ordered by delivery time (then id).
    fn from(chain: MessageChain) -> Self {
        let mut nodes: Vec<ChainNodeView> = chain.entries.into_iter().map(Into::into).collect();
        nodes.sort_by(|a, b| a.deliver_at.cmp(&b.deliver_at).then_with(|| a.id.cmp(&b.id)));
        Self { focus: chain.focus, nodes, truncated: chain.truncated }
    }
}

impl From<ActorSnapshot> for ActorView {
    fn from(s: ActorSnapshot) -> Self {
        Self {
            uri: s.address,
            template: s.template.as_str().to_owned(),
            params: s.params,
            state: s.state,
            version: s.version,
            created_at: s.created_at,
        }
    }
}

impl From<&RegisteredType> for TypeView {
    fn from(t: &RegisteredType) -> Self {
        Self {
            template: t.template().as_str().to_owned(),
            hash: t.definition.contract_hash.clone(),
            state: t.contract.state_schema().cloned(),
            messages: t.contract.message_schemas().clone(),
        }
    }
}

impl From<&ResolvedActorType> for TypeMatchView {
    fn from(r: &ResolvedActorType) -> Self {
        Self { actor_type: (&r.registered).into(), params: r.params.clone() }
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
    pub fn delivery(&self) -> Result<Delivery, String> {
        match (self.deliver_at, self.delay) {
            (Some(_), Some(_)) => Err("Give either deliverAt or delay, not both".to_owned()),
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
