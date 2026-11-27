//! The actor pane's writes, bound to the pane that offered them.

use cicada::ActorAddress;
use cicada_client::{ActorClient, ClientResult};
use tokio::{sync::watch, task::JoinHandle};

use super::{Actors, PaneView, Shared, State as ActorState};
use crate::pager::Paged;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Pause,
    Resume,
    CancelTurn,
    Purge,
    Delete,
}

impl Action {
    fn asks_first(self) -> bool {
        matches!(self, Self::CancelTurn | Self::Purge | Self::Delete)
    }

    async fn call(self, client: &ActorClient, address: &ActorAddress) -> ClientResult<Option<u64>> {
        match self {
            Self::Pause => client.pause_mailbox(address).await.map(acknowledged),
            Self::Resume => client.resume_mailbox(address).await.map(acknowledged),
            Self::CancelTurn => client.cancel_turn(address).await.map(acknowledged),
            Self::Purge => client.purge_mailbox(address).await,
            Self::Delete => client.delete_actor(address).await.map(acknowledged),
        }
    }
}

fn acknowledged(answer: bool) -> Option<u64> {
    answer.then_some(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Feedback {
    SelectionChanged,
    Absent(Action),
    Succeeded(Action, u64),
    Refused,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct View {
    pub busy: bool,
    pub feedback: Option<(u64, Feedback)>,
}

struct Target {
    generation: u64,
    address: ActorAddress,
}

impl Target {
    fn matches(&self, state: &ActorState) -> bool {
        self.generation == state.pane_generation
            && state.selected.as_ref().is_some_and(|path| path.address() == &self.address)
    }
}

struct Pending {
    id: u64,
    action: Action,
    target: Target,
    task: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub(super) struct State {
    serial: u64,
    pending: Option<Pending>,
}

#[derive(Debug)]
pub(crate) struct Confirmation {
    pub id: u64,
}

impl Actors {
    pub(crate) fn actions(&self) -> watch::Receiver<View> {
        self.shared.actions.subscribe()
    }

    /// Presses a button from the rendered pane. A destructive action reserves the slot until the sheet ends.
    pub(crate) fn press_action(&self, pane: &PaneView, action: Action) -> Option<Confirmation> {
        let mut state = self.shared.lock();
        if state.actions.pending.is_some() || state.client.is_none() {
            return None;
        }
        let address = pane.address.clone()?;
        let target = Target { generation: pane.generation, address };
        if !target.matches(&state) {
            self.shared.finish_action(&mut state, Some(Feedback::SelectionChanged));
            return None;
        }
        state.actions.serial += 1;
        let id = state.actions.serial;
        state.actions.pending = Some(Pending { id, action, target, task: None });
        self.shared.actions.send_replace(View { busy: true, feedback: None });
        if action.asks_first() {
            Some(Confirmation { id })
        } else {
            self.shared.send_action(&mut state);
            None
        }
    }

    pub(crate) fn answer_action(&self, id: u64, confirmed: bool) {
        let mut state = self.shared.lock();
        let Some(pending) = &state.actions.pending else { return };
        if pending.id != id || pending.task.is_some() {
            return;
        }
        if !confirmed {
            self.shared.finish_action(&mut state, None);
        } else if !pending.target.matches(&state) {
            self.shared.finish_action(&mut state, Some(Feedback::SelectionChanged));
        } else {
            self.shared.send_action(&mut state);
        }
    }
}

impl Shared {
    pub(super) fn clear_actions(&self, state: &mut ActorState) {
        let task = state.actions.pending.take().and_then(|pending| pending.task);
        state.abort(task);
        self.actions.send_replace(View::default());
    }

    fn finish_action(&self, state: &mut ActorState, feedback: Option<Feedback>) {
        state.actions.pending = None;
        self.actions
            .send_replace(View { busy: false, feedback: feedback.map(|feedback| (state.pane_generation, feedback)) });
    }

    fn send_action(self: &std::sync::Arc<Self>, state: &mut ActorState) {
        let pending = state.actions.pending.as_mut().expect("the action owns the slot");
        let id = pending.id;
        let shared = self.clone();
        pending.task = Some(self.handle.spawn(async move {
            let (client, action, address) = {
                let mut state = shared.lock();
                let Some(pending) = state.actions.pending.as_ref().filter(|pending| pending.id == id) else { return };
                if !pending.target.matches(&state) {
                    shared.finish_action(&mut state, Some(Feedback::SelectionChanged));
                    return;
                }
                (state.client.clone().expect("the action has a client"), pending.action, pending.target.address.clone())
            };
            let answer = action.call(&client, &address).await;
            let mut state = shared.lock();
            let Some(pending) = state.actions.pending.as_ref().filter(|pending| pending.id == id) else { return };
            let feedback = pending.target.matches(&state).then(|| match answer {
                Ok(Some(count)) => Feedback::Succeeded(action, count),
                Ok(None) => Feedback::Absent(action),
                Err(_) => Feedback::Refused,
            });
            shared.finish_action(&mut state, feedback);
        }));
    }
}
