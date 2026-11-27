//! The actor pane's Send Message sheet: the message types of the actor's type, a JSON payload checked locally against
//! the chosen type's schema, and the send. The sheet holds the action slot from opening to closing, so no other action
//! runs while it is open.

mod check;

use std::sync::Arc;
use std::time::Duration;

use cicada::api::{TypeView, problem_types};
use cicada::store::MessageId;
use cicada::turn::SendOptions;
use cicada_client::ClientError;
use serde_json::Value;
use tokio::{sync::watch, task::JoinHandle, time::Instant, time::timeout};

use crate::json::Json;
use check::{Checker, Job, Land, Landed};
pub(crate) use check::{MAX_TEXT, Verdict};

use super::{Alert, Feedback, Target, View};
use crate::actors::{Actors, PaneView, Read, Shared, State as ActorState};
use crate::model::{Part, TIMED_OUT, within};
use crate::pager::Paged;
use crate::types::Types;

/// The message types offered in the sheet's pop-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Choices {
    /// `/types` is being read.
    Loading,
    /// The read of `/types` failed: why. Retry reads it again.
    Failed(String),
    /// The server has no contract for the actor's type.
    NotRegistered,
    /// The type's contract has no message types: nothing can be sent.
    NoMessages,
    /// The message types in name order.
    Ready(Vec<String>),
}

impl Choices {
    /// The text shown instead of the pop-up, if any.
    pub fn note(&self) -> Option<String> {
        match self {
            Self::Loading => Some("Loading message types…".into()),
            Self::Failed(error) => Some(format!("The message types could not be read: {error}")),
            Self::NotRegistered => Some("type not registered".into()),
            Self::NoMessages => Some("this type takes no messages".into()),
            Self::Ready(_) => None,
        }
    }
}

/// What the Send Message sheet shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SendView {
    /// The sheet: the action serial it holds.
    pub id: u64,
    /// The actor the message goes to.
    pub address: cicada::ActorAddress,
    /// Its type.
    pub type_name: String,
    /// The message types of the pop-up.
    pub choices: Choices,
    /// The chosen message type's index in [`Choices::Ready`].
    pub chosen: usize,
    /// The chosen message type's schema, formatted, once the check has compiled it.
    pub schema: Option<Arc<Json>>,
    /// False when the chosen schema cannot be compiled: the server does not check its payloads.
    pub compiles: bool,
    /// The local check of the payload in the editor.
    pub verdict: Verdict,
    /// The id Send uses: the same until the message type or the parsed payload changes, so that a second Send after
    /// a lost answer is not delivered twice.
    pub message_id: MessageId,
    /// True while a send is out.
    pub sending: bool,
    /// The last send's refusal shown in the sheet: the server's 400 or 422 text, an absent actor or a timeout.
    pub refusal: Option<Alert>,
    /// The last send's refusal shown as an alert over the sheet, such as a 413.
    pub alert: Option<Alert>,
    /// Grows at every Send, so the view shows each alert once.
    pub sends: u64,
}

impl SendView {
    /// The note under the schema when the server does not check this type's payloads.
    pub fn schema_note(&self) -> Option<&'static str> {
        (!self.compiles).then_some(Verdict::UNCHECKED)
    }

    /// True when Send is enabled: a message type is chosen, the payload passed the local check (or its schema cannot
    /// be compiled) and no send is out.
    pub fn can_send(&self) -> bool {
        !self.sending
            && matches!(self.choices, Choices::Ready(_))
            && matches!(self.verdict, Verdict::Passes | Verdict::Unchecked)
    }

    fn message_type(&self) -> Option<&str> {
        match &self.choices {
            Choices::Ready(names) => names.get(self.chosen).map(String::as_str),
            _ => None,
        }
    }
}

/// How long the editor must be still before its payload is checked.
pub(crate) const CHECK_DELAY: Duration = Duration::from_millis(300);
/// The payload the editor starts with.
pub(crate) const START: &str = "{}";

/// The open sheet's state beside its view.
pub(super) struct Sheet {
    view: SendView,
    target: Target,
    row: Option<Arc<TypeView>>,
    types_task: Option<JoinHandle<()>>,
    /// `None` when its thread could not be started: nothing passes the check.
    checker: Option<Checker>,
    /// Grows at each choice of a message type.
    choice: u64,
    /// Grows at each edit and choice.
    edit: u64,
    schema: Option<Arc<Value>>,
    text: Arc<str>,
    delay: Option<JoinHandle<()>>,
    /// The parsed payload the verdict let through, sent as is.
    checked: Option<Arc<Value>>,
    /// The message type and parsed payload that [`SendView::message_id`] was last used for.
    identified: Option<(String, Arc<Value>)>,
    send_task: Option<JoinHandle<()>>,
}

impl Actors {
    /// The open Send Message sheet, if any.
    pub(crate) fn send_sheet(&self) -> watch::Receiver<Option<SendView>> {
        self.shared.send.subscribe()
    }

    /// Send Message… from the rendered pane: opens the sheet on the actor shown, with the message types from the Types
    /// page's list when it holds the type, else from one read of `/types`. Returns the sheet's id.
    pub(crate) fn open_send(&self, pane: &PaneView, types: &Types) -> Option<u64> {
        let Read::Ready(Some(details)) = &pane.actor else { return None };
        let cached = types.cached(&details.type_name);
        let mut state = self.shared.lock();
        if state.actions.taken() || state.client.is_none() {
            return None;
        }
        let target = Target { generation: pane.generation, address: pane.address.clone()? };
        if !target.matches(&state) {
            let feedback = Feedback::selection_changed(state.pane_generation);
            self.shared.finish_action(&mut state, Some(feedback));
            return None;
        }
        state.actions.serial += 1;
        let id = state.actions.serial;
        let mut view = SendView {
            id,
            address: target.address.clone(),
            type_name: details.type_name.clone(),
            choices: Choices::Loading,
            chosen: 0,
            schema: None,
            compiles: true,
            verdict: Verdict::Checking,
            message_id: MessageId::random(),
            sending: false,
            refusal: None,
            alert: None,
            sends: 0,
        };
        let checker = Checker::start(Arc::downgrade(&self.shared));
        if checker.is_none() {
            view.verdict = Verdict::Refused("the check could not start".into());
        }
        state.actions.sheet = Some(Sheet {
            view,
            target,
            row: None,
            types_task: None,
            checker,
            choice: 0,
            edit: 0,
            schema: None,
            text: START.into(),
            delay: None,
            checked: None,
            identified: None,
            send_task: None,
        });
        self.shared.actions.send_replace(View { busy: true, serial: id, feedback: None });
        match cached {
            Some(row) => self.shared.land_row(&mut state, Some(row)),
            None => self.shared.read_types(&mut state),
        }
        Some(id)
    }

    /// Reads `/types` again after a failed read.
    pub(crate) fn retry_send_types(&self, id: u64) {
        let mut state = self.shared.lock();
        if state
            .actions
            .sheet
            .as_ref()
            .is_some_and(|sheet| sheet.view.id == id && matches!(sheet.view.choices, Choices::Failed(_)))
        {
            self.shared.read_types(&mut state);
        }
    }

    /// Chooses the message type at `index` of the pop-up: its schema is compiled and the payload checked at once.
    pub(crate) fn choose_send_type(&self, id: u64, index: usize) {
        let mut state = self.shared.lock();
        let Some(sheet) = state.actions.sheet.as_mut().filter(|sheet| sheet.view.id == id) else { return };
        if matches!(&sheet.view.choices, Choices::Ready(names) if index < names.len() && index != sheet.view.chosen) {
            sheet.view.chosen = index;
            sheet.choose();
            self.shared.publish_sheet(&state);
        }
    }

    /// An edit of the payload: Send is off at once, and the payload is checked once the editor has been still for
    /// [`CHECK_DELAY`].
    pub(crate) fn edit_send_payload(&self, id: u64, text: &str) {
        let mut state = self.shared.lock();
        let Some(sheet) = state.actions.sheet.as_mut().filter(|sheet| sheet.view.id == id) else { return };
        sheet.text = text.into();
        sheet.changed();
        let edit = sheet.edit;
        let (shared, due) = (self.shared.clone(), Instant::now() + CHECK_DELAY);
        let task = self.shared.handle.spawn(async move {
            tokio::time::sleep_until(due).await;
            let mut state = shared.lock();
            if let Some(sheet) = state.actions.sheet.as_mut().filter(|sheet| sheet.view.id == id && sheet.edit == edit)
            {
                sheet.delay = None;
                sheet.post();
            }
        });
        let old = state.actions.sheet.as_mut().and_then(|sheet| sheet.delay.replace(task));
        state.abort(old);
        self.shared.publish_sheet(&state);
    }

    /// Send: sends the checked payload with the sheet's message id. A success closes the sheet with a notice and reads
    /// the pane again; a refusal keeps the sheet and the payload.
    pub(crate) fn send_message(&self, id: u64) {
        let mut state = self.shared.lock();
        let Some(sheet) = state.actions.sheet.as_ref().filter(|sheet| sheet.view.id == id) else { return };
        let (Some(value), Some(message_type)) = (sheet.checked.clone(), sheet.view.message_type()) else { return };
        if !sheet.view.can_send() {
            return;
        }
        if !sheet.target.matches(&state) {
            let feedback = Feedback::selection_changed(state.pane_generation);
            self.shared.drop_sheet(&mut state);
            self.shared.finish_action(&mut state, Some(feedback));
            return;
        }
        let Some(client) = state.client.clone() else { return };
        let (message_type, address) = (message_type.to_owned(), sheet.target.address.clone());
        let options = SendOptions { id: Some(sheet.view.message_id.clone()), ..SendOptions::default() };
        let sheet = state.actions.sheet.as_mut().expect("the sheet is open");
        sheet.view.sending = true;
        sheet.view.refusal = None;
        sheet.view.alert = None;
        sheet.view.sends += 1;
        let sends = sheet.view.sends;
        let shared = self.shared.clone();
        let deadline = self.shared.limits.deadline;
        let task = self.shared.handle.spawn(async move {
            let payload = Value::clone(&value);
            let answer = timeout(deadline, client.send_message(&address, &message_type, payload, options)).await;
            let mut state = shared.lock();
            if !state.actions.sheet.as_ref().is_some_and(|sheet| sheet.view.id == id && sheet.view.sends == sends) {
                return;
            }
            shared.land_send(&mut state, answer.ok());
        });
        let old = state.actions.sheet.as_mut().and_then(|sheet| sheet.send_task.replace(task));
        state.abort(old);
        self.shared.publish_sheet(&state);
    }

    /// Cancel: closes the sheet and drops what it was doing.
    pub(crate) fn cancel_send(&self, id: u64) {
        let mut state = self.shared.lock();
        if state.actions.sheet.as_ref().is_some_and(|sheet| sheet.view.id == id) {
            self.shared.drop_sheet(&mut state);
            self.shared.finish_action(&mut state, None);
        }
    }
}

impl Shared {
    /// Closes the sheet, if one is open, and cancels its work; the caller frees the slot.
    pub(super) fn drop_sheet(&self, state: &mut ActorState) {
        if let Some(sheet) = state.actions.sheet.take() {
            state.abort(sheet.types_task);
            state.abort(sheet.delay);
            state.abort(sheet.send_task);
            self.send.send_replace(None);
        }
    }

    fn publish_sheet(&self, state: &ActorState) {
        self.send.send_replace(state.actions.sheet.as_ref().map(|sheet| sheet.view.clone()));
    }

    fn read_types(self: &Arc<Self>, state: &mut ActorState) {
        let Some(client) = state.client.clone() else { return };
        let Some(sheet) = state.actions.sheet.as_mut() else { return };
        let id = sheet.view.id;
        sheet.view.choices = Choices::Loading;
        let shared = self.clone();
        let deadline = self.limits.deadline;
        let task = self.handle.spawn(async move {
            let answer = within(Instant::now() + deadline, client.types()).await;
            let mut state = shared.lock();
            let Some(sheet) = state.actions.sheet.as_mut().filter(|sheet| sheet.view.id == id) else { return };
            sheet.types_task = None;
            match answer {
                Part::Ready(rows) => {
                    let row = rows.into_iter().find(|row| row.name == sheet.view.type_name).map(Arc::new);
                    shared.land_row(&mut state, row);
                }
                Part::Failed(error) => {
                    sheet.view.choices = Choices::Failed(error);
                    shared.publish_sheet(&state);
                }
            }
        });
        let old = state.actions.sheet.as_mut().and_then(|sheet| sheet.types_task.replace(task));
        state.abort(old);
        self.publish_sheet(state);
    }

    /// Lands the answer to the sheet's current send; `None` when it timed out.
    fn land_send(
        self: &Arc<Self>,
        state: &mut ActorState,
        answer: Option<cicada_client::ClientResult<cicada_client::SendReceipt>>,
    ) {
        let selected = state.actions.sheet.as_ref().is_some_and(|sheet| sheet.target.matches(state));
        let sheet = state.actions.sheet.as_mut().expect("the caller checked the sheet");
        sheet.send_task = None;
        sheet.view.sending = false;
        match answer {
            Some(Ok(receipt)) => {
                let text = format!("{}sent {}", if receipt.duplicate { "already " } else { "" }, receipt.id);
                self.drop_sheet(state);
                self.refresh_after_write(state);
                let feedback = selected.then(|| Feedback::Notice { generation: state.pane_generation, text });
                self.finish_action(state, feedback);
                return;
            }
            Some(Err(ClientError::Api(problem))) if problem.problem_type == problem_types::NO_SUCH_INSTANCE => {
                sheet.view.refusal = Some(Alert { title: "actor not found".into(), detail: None });
            }
            Some(Err(error)) => {
                let in_sheet = matches!(error.status(), Some(400 | 422));
                let alert = state.actions.redactions.alert(error);
                let sheet = state.actions.sheet.as_mut().expect("the sheet is open");
                if in_sheet {
                    sheet.view.refusal = Some(alert);
                } else {
                    sheet.view.alert = Some(alert);
                }
            }
            None => {
                sheet.view.refusal = Some(Alert {
                    title: TIMED_OUT.into(),
                    detail: Some("The server may have the message. Send again sends the same message id.".into()),
                });
                // The message may have reached the server: read the pane again, as after an action that timed out.
                self.refresh_after_write(state);
            }
        }
        self.publish_sheet(state);
    }

    fn land_row(&self, state: &mut ActorState, row: Option<Arc<TypeView>>) {
        let Some(sheet) = state.actions.sheet.as_mut() else { return };
        sheet.view.choices = match &row {
            None => Choices::NotRegistered,
            Some(row) if row.messages.is_empty() => Choices::NoMessages,
            Some(row) => Choices::Ready(row.messages.keys().cloned().collect()),
        };
        sheet.view.chosen = 0;
        sheet.row = row;
        if matches!(sheet.view.choices, Choices::Ready(_)) {
            sheet.choose();
        }
        self.publish_sheet(state);
    }
}

impl Sheet {
    /// A new payload or message type: no verdict until it is checked.
    fn changed(&mut self) {
        self.edit += 1;
        self.checked = None;
        self.view.refusal = None;
        self.view.alert = None;
        if self.checker.is_some() {
            self.view.verdict = Verdict::Checking;
        }
    }

    /// The chosen message type changed: compile its schema and check the payload now.
    fn choose(&mut self) {
        self.choice += 1;
        self.schema =
            self.row.as_ref().and_then(|row| row.messages.values().nth(self.view.chosen)).cloned().map(Arc::new);
        self.view.schema = None;
        self.view.compiles = true;
        self.changed();
        self.post();
    }

    fn post(&self) {
        let (Some(checker), Some(schema)) = (&self.checker, &self.schema) else { return };
        checker.post(Job {
            sheet: self.view.id,
            choice: self.choice,
            edit: self.edit,
            schema: schema.clone(),
            text: self.text.clone(),
        });
    }
}

impl Land for Shared {
    fn land(&self, landed: Landed) {
        let mut state = self.lock();
        let Some(sheet) = state.actions.sheet.as_mut() else { return };
        match landed {
            Landed::Schema { sheet: id, choice, json, compiles } if id == sheet.view.id && choice == sheet.choice => {
                sheet.view.schema = Some(json);
                sheet.view.compiles = compiles;
            }
            Landed::Verdict { sheet: id, choice, edit, verdict, value }
                if id == sheet.view.id && choice == sheet.choice && edit == sheet.edit =>
            {
                sheet.view.verdict = verdict;
                if let (Some(value), Some(name)) = (&value, sheet.view.message_type().map(str::to_owned)) {
                    match &sheet.identified {
                        Some((used, payload)) if *used == name && payload == value => {}
                        Some(_) => sheet.view.message_id = MessageId::random(),
                        None => {}
                    }
                    sheet.identified = Some((name, value.clone()));
                }
                sheet.checked = value;
            }
            _ => return,
        }
        self.publish_sheet(&state);
    }
}

#[cfg(test)]
impl Actors {
    /// Holds the sheet's next check until the test releases it.
    pub(crate) fn hold_send_check(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (arrived, arrival) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let state = self.shared.lock();
        let checker = state.actions.sheet.as_ref().and_then(|sheet| sheet.checker.as_ref()).expect("an open sheet");
        checker.holds.lock().unwrap().push_back((arrived, released));
        (arrival, release)
    }
}
