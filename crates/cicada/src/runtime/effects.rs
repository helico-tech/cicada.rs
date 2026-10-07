//! Buffered effects of a turn and raw sends checked against the registry.

use serde_json::Value;

use super::limits;
use super::resolver::TypeResolver;
use crate::address::{ActorAddress, UriTemplate};
use crate::plugin::{HookSend, TurnInfo};
use crate::store::{ActorCreate, MessageId, OutgoingMessage};
use crate::turn::{TurnError, check_delivery, refuse_reserved};

/// Buffered effects of a turn, applied by its commit.
#[derive(Debug, Default)]
pub(crate) struct EffectBuffer {
    pub creates: Vec<ActorCreate>,
    pub sends: Vec<OutgoingMessage>,
    /// Pending messages of this instance to cancel at commit.
    pub cancels: Vec<MessageId>,
    pub stop: bool,
    /// The template of the ref type each typed send was addressed with; checked against the registry at the end.
    pub typed_targets: Vec<(ActorAddress, UriTemplate)>,
}

/// A point to roll an [`EffectBuffer`] back to.
pub(crate) struct Mark {
    creates: usize,
    sends: usize,
    cancels: usize,
    stop: bool,
}

impl EffectBuffer {
    pub fn mark(&self) -> Mark {
        Mark { creates: self.creates.len(), sends: self.sends.len(), cancels: self.cancels.len(), stop: self.stop }
    }

    pub fn roll_back(&mut self, mark: &Mark) {
        self.creates.truncate(mark.creates);
        self.sends.truncate(mark.sends);
        self.cancels.truncate(mark.cancels);
        self.stop = mark.stop;
    }
}

/// Raw sends shared by handlers and hook contexts: checked against the registry when called.
pub struct RawSender<'a> {
    pub(crate) resolver: &'a TypeResolver,
    pub(crate) effects: &'a mut EffectBuffer,
    pub(crate) allow_reserved: bool,
}

impl RawSender<'_> {
    pub(crate) async fn send(
        &mut self,
        info: &TurnInfo,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        options: HookSend,
    ) -> Result<MessageId, TurnError> {
        if !self.allow_reserved {
            refuse_reserved(&options.headers)?;
        }
        check_delivery(&options.delivery)?;
        let id = options.id.unwrap_or_else(MessageId::random);
        limits::check_envelope(&id, message_type, &payload, options.correlation_id.as_deref(), &options.headers)
            .map_err(TurnError::Invalid)?;
        self.resolver.check_send(to, message_type).await?;
        self.effects.sends.push(OutgoingMessage {
            to: to.clone(),
            message_type: message_type.to_owned(),
            payload,
            id: id.clone(),
            from: Some(info.self_address.clone()),
            delivery: options.delivery,
            correlation_id: options.correlation_id,
            headers: options.headers,
            retry: options.retry,
            caused_by: Some(info.message.id.clone()),
            retention: options.retention,
        });
        Ok(id)
    }
}
