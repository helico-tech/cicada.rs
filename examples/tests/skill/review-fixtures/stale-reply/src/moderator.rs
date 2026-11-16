// The single moderator: answers each Check with Approved or Denied to the room by its name.

use std::collections::BTreeSet;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

use crate::rooms::{Moderator, ModeratorMsg, Room, RoomMsg};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ModeratorState {
    pub banned: BTreeSet<String>,
}

pub struct ModeratorActor;

impl Actor for ModeratorActor {
    type Ref = Moderator;
    type State = ModeratorState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ModeratorMsg) -> HandlerResult {
        match message {
            ModeratorMsg::Check { room, user } => {
                let reply = if turn.state().banned.contains(&user) {
                    RoomMsg::Denied { user }
                } else {
                    RoomMsg::Approved { user }
                };
                turn.send(&Room { name: room }, reply)?;
            }
        }
        Ok(())
    }
}
