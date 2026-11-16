// Sends the shipping email of one order through a hosted email service ("Postbird").

use std::time::Duration;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use crate::postbird::Postbird;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notifier {
    pub order: String,
}

impl ActorRef for Notifier {
    const TYPE: &'static str = "notifiers";
    type Message = NotifierMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum NotifierMsg {
    Shipped { email: String, tracking: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct NotifierState {
    pub sent: bool,
}

pub struct NotifierActor {
    pub postbird: Postbird,
}

impl Actor for NotifierActor {
    type Ref = Notifier;
    type State = NotifierState;
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(30));

    async fn handle(&self, turn: &mut Turn<Self>, message: NotifierMsg) -> HandlerResult {
        match message {
            NotifierMsg::Shipped { email, tracking } => {
                if turn.state().sent {
                    return Ok(());
                }
                // Postbird takes a client reference; we pass the message id so that a repeated attempt reuses it.
                let reference = turn.message().id.to_string();
                self.postbird.send_template("shipped", &email, &[("tracking", &tracking)], &reference).await?;
                turn.state_mut().sent = true;
            }
        }
        Ok(())
    }
}
