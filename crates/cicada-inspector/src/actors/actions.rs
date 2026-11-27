//! The actor pane's writes, bound to the pane that offered them.

mod alerts;

use std::sync::Arc;

use cicada::ActorAddress;
use cicada_client::{ActorClient, ClientResult};
use tokio::{sync::watch, task::JoinHandle, time::timeout};

use super::{Actors, Children, Paging, PaneView, Read, Shared, State as ActorState};
use crate::model::TIMED_OUT;
use crate::pager::Paged;
pub(crate) use alerts::{Alert, Redactions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Pause,
    Resume,
    CancelTurn,
    Purge,
    Delete,
}

impl Action {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Pause => "Pause",
            Self::Resume => "Resume",
            Self::CancelTurn => "Cancel Turn",
            Self::Purge => "Purge Mailbox",
            Self::Delete => "Delete",
        }
    }

    pub(crate) fn offered(pane: &PaneView) -> impl Iterator<Item = Self> {
        let available = matches!(pane.actor, Read::Ready(Some(_)));
        let toggle = match &pane.actor {
            Read::Ready(Some(details)) if details.paused => Self::Resume,
            _ => Self::Pause,
        };
        [
            available.then_some(toggle),
            (available && matches!(pane.turn, Read::Ready(Some(_)))).then_some(Self::CancelTurn),
            available.then_some(Self::Purge),
            available.then_some(Self::Delete),
        ]
        .into_iter()
        .flatten()
    }

    pub(crate) fn asks_first(self) -> bool {
        matches!(self, Self::CancelTurn | Self::Purge | Self::Delete)
    }

    fn confirmation(self, pane: &PaneView, address: &ActorAddress, children: Option<&Children>) -> Option<String> {
        match self {
            Self::Pause | Self::Resume => None,
            Self::CancelTurn => {
                let Read::Ready(Some(turn)) = &pane.turn else { return None };
                Some(format!(
                    "Cancel the turn running for {address} (message {} as last read)? \
                     The server cancels whichever turn is running when it receives this.",
                    turn.message_id
                ))
            }
            Self::Purge => {
                let pending = match &pane.mailbox {
                    Read::Ready(mailbox) if pane.errors.mailbox.is_none() => {
                        format!("{}{} pending, as last read", lower_bound(mailbox.more), pane.pending().count())
                    }
                    _ => "not read".into(),
                };
                Some(format!("Purge the mailbox of {address} ({pending})? The running message, if any, is not purged."))
            }
            Self::Delete => {
                let children = match children {
                    Some(page) if page.more || !page.rows.is_empty() || page.paging == Paging::Idle => {
                        format!("{}{} children", lower_bound(page.more), page.rows.len())
                    }
                    _ => "children not read".into(),
                };
                Some(format!(
                    "Delete {address} ({children})? This removes this actor and every descendant, with all their mailboxes."
                ))
            }
        }
    }

    fn notice(self, count: Option<u64>) -> String {
        match (self, count) {
            (Self::CancelTurn, None) => "no turn was running".into(),
            (_, None) => "actor not found".into(),
            (Self::Pause, Some(_)) => "paused".into(),
            (Self::Resume, Some(_)) => "resumed".into(),
            (Self::CancelTurn, Some(_)) => "cancel requested".into(),
            (Self::Purge, Some(count)) => format!("purged {count} messages"),
            (Self::Delete, Some(_)) => "deleted".into(),
        }
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

fn lower_bound(more: bool) -> &'static str {
    if more { "at least " } else { "" }
}

fn acknowledged(answer: bool) -> Option<u64> {
    answer.then_some(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Feedback {
    Notice(String),
    Alert(Alert),
}

impl Feedback {
    fn selection_changed() -> Self {
        Self::Notice("selection changed; nothing done".into())
    }
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
    pub(super) redactions: Redactions,
}

#[derive(Debug)]
pub(crate) struct Confirmation {
    pub id: u64,
    pub action: Action,
    pub text: String,
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
            self.shared.finish_action(&mut state, Some(Feedback::selection_changed()));
            return None;
        }
        if !Action::offered(pane).any(|offered| offered == action) {
            return None;
        }
        let text = {
            let list = self.shared.list.borrow();
            let children = state.selected.as_ref().and_then(|path| list.children.get(path));
            action.confirmation(pane, &target.address, children)
        };
        state.actions.serial += 1;
        let id = state.actions.serial;
        state.actions.pending = Some(Pending { id, action, target, task: None });
        self.shared.actions.send_replace(View { busy: true, feedback: None });
        if let Some(text) = text {
            Some(Confirmation { id, action, text })
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
            self.shared.finish_action(&mut state, Some(Feedback::selection_changed()));
        } else {
            self.shared.send_action(&mut state);
        }
    }
}

impl Shared {
    pub(super) fn clear_actions(&self, state: &mut ActorState) {
        let task = state.actions.pending.take().and_then(|pending| pending.task);
        state.abort(task);
        state.actions.redactions = Redactions::default();
        self.actions.send_replace(View::default());
    }

    fn finish_action(&self, state: &mut ActorState, feedback: Option<Feedback>) {
        state.actions.pending = None;
        self.actions
            .send_replace(View { busy: false, feedback: feedback.map(|feedback| (state.pane_generation, feedback)) });
    }

    fn send_action(self: &Arc<Self>, state: &mut ActorState) {
        let pending = state.actions.pending.as_mut().expect("the action owns the slot");
        let id = pending.id;
        let shared = self.clone();
        pending.task = Some(self.handle.spawn(async move {
            let (client, action, address) = {
                let mut state = shared.lock();
                let Some(pending) = state.actions.pending.as_ref().filter(|pending| pending.id == id) else { return };
                if !pending.target.matches(&state) {
                    shared.finish_action(&mut state, Some(Feedback::selection_changed()));
                    return;
                }
                (state.client.clone().expect("the action has a client"), pending.action, pending.target.address.clone())
            };
            let answer = timeout(shared.limits.deadline, action.call(&client, &address)).await;
            let mut state = shared.lock();
            let Some(pending) = state.actions.pending.as_ref().filter(|pending| pending.id == id) else { return };
            let feedback = pending.target.matches(&state).then(|| match answer {
                Ok(Ok(count)) => Feedback::Notice(action.notice(count)),
                Ok(Err(error)) => Feedback::Alert(state.actions.redactions.alert(error)),
                Err(_) => Feedback::Alert(Alert { title: TIMED_OUT.into(), detail: None }),
            });
            shared.finish_action(&mut state, feedback);
        }));
    }
}
