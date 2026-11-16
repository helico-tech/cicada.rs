// Chat rooms keyed by their name. A room asks the moderator before a user joins.
// Closing a room deletes it; the same name can be opened again later, also as a private room.

use std::collections::BTreeSet;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    pub name: String,
}

impl ActorRef for Room {
    const TYPE: &'static str = "rooms";
    type Message = RoomMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum RoomMsg {
    RequestJoin { user: String },
    Approved { user: String },
    Denied { user: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RoomState {
    pub private: bool,
    pub members: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moderator;

impl ActorRef for Moderator {
    const TYPE: &'static str = "moderator";
    type Message = ModeratorMsg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ModeratorMsg {
    /// Checks the user against the ban list. One moderator serves every room; at peak its queue is minutes long.
    Check { room: String, user: String },
}

pub struct RoomActor;

impl Actor for RoomActor {
    type Ref = Room;
    type State = RoomState;

    async fn handle(&self, turn: &mut Turn<Self>, message: RoomMsg) -> HandlerResult {
        match message {
            RoomMsg::RequestJoin { user } => {
                let room = turn.self_ref().name.clone();
                turn.send(&Moderator, ModeratorMsg::Check { room, user })?;
            }
            RoomMsg::Approved { user } => {
                turn.state_mut().members.insert(user);
            }
            RoomMsg::Denied { .. } => {}
        }
        Ok(())
    }
}

/// Host code behind the HTTP API: closing deletes the room actor.
pub async fn close_room(system: &ActorSystem, name: &str) -> anyhow::Result<()> {
    system.delete(&Room { name: name.to_owned() }).await?;
    Ok(())
}

/// Host code behind the HTTP API: opening creates the room actor under its name.
pub async fn open_room(system: &ActorSystem, name: &str, private: bool) -> anyhow::Result<()> {
    let state = RoomState { private, ..RoomState::default() };
    system.create(&Room { name: name.to_owned() }, &state).await?;
    Ok(())
}
