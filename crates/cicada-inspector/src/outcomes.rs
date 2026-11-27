//! The Outcomes page: the finished messages under a prefix and of one status, read a page at a time through the root
//! pager, each message once per generation; the failed count of the sidebar; and the pane of the selected outcome or
//! of a message reached by a link of its cause chain, with the messages it caused. The list's generation grows at every
//! key press in the search field, at a switch of the status, on Refresh and on a context switch; the pane's whenever it
//! shows another message, on Refresh and on a context switch.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use cicada::ActorAddress;
use cicada::api::{MessageState, MessageStatusView, MessageView, OutcomeView, StatsView};
use cicada::retry::{Backoff, RetryPolicy};
use cicada::store::{MessageId, OutcomeStatus};
use cicada::time::Timestamp;
use cicada_client::ActorClient;
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::json::{self, Json};
use crate::model::{Limits, Part, within};
use crate::pager::{Cursor, Landed, NEAR_END, PAGE, Paged, Pager, Paging, SoftPaged};
use crate::refresh::{Budget, Done, RefreshPage, Stale};

/// The messages of one page of "Caused".
pub const CAUSED_PAGE: usize = 50;
/// The tooltip of the sidebar's failed count.
pub const COUNT_TIP: &str = "failed, kept by retention, as of the last read";

/// The status switch of the toolbar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusFilter {
    /// Failed outcomes, the default.
    #[default]
    Failed,
    /// Cancelled outcomes.
    Cancelled,
    /// Done outcomes.
    Done,
    /// Every outcome, purged ones included, with a status column.
    All,
}

impl StatusFilter {
    /// The switch's choices, in its order.
    pub const ALL: [Self; 4] = [Self::Failed, Self::Cancelled, Self::Done, Self::All];

    /// The switch's text.
    pub fn title(self) -> &'static str {
        match self {
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Done => "Done",
            Self::All => "All",
        }
    }

    /// The status asked of `/outcomes`; `None` asks for every status.
    pub fn status(self) -> Option<OutcomeStatus> {
        match self {
            Self::Failed => Some(OutcomeStatus::Failed),
            Self::Cancelled => Some(OutcomeStatus::Cancelled),
            Self::Done => Some(OutcomeStatus::Done),
            Self::All => None,
        }
    }
}

/// What the Outcomes table shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutcomesView {
    /// The prefix of the actor addresses the rows are under.
    pub prefix: String,
    /// The status the rows have; [`StatusFilter::All`] shows a status column.
    pub status: StatusFilter,
    /// Grows on a new list, prefix, status, Refresh or context.
    pub generation: u64,
    /// The outcomes read so far, each message once, in the server's order; shared with the pane and later views.
    pub rows: Vec<Arc<OutcomeView>>,
    /// Whether a page is asked for or failed.
    pub paging: Paging,
    /// The mark on rows carried over by a soft refresh.
    pub stale: Option<Stale>,
    /// True when the server has rows past these.
    pub more: bool,
}

impl OutcomesView {
    /// True when the view should ask for the next page with `last` the last visible row: it is within [`NEAR_END`]
    /// of the end, the server has more, and no page is asked for or failed.
    pub fn wants_more(&self, last: usize) -> bool {
        self.more && self.paging == Paging::Idle && last + NEAR_END >= self.rows.len()
    }

    /// The row of the message `id`.
    pub fn index(&self, id: &MessageId) -> Option<usize> {
        self.rows.iter().position(|row| &row.id == id)
    }
}

/// The failed count of the sidebar: `failed` of `/stats` with no prefix.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailedCount {
    /// The last answer: `None` before the first, `Some(None)` from an older server that sends no count.
    pub value: Option<Option<u64>>,
    /// The error of the last read, when it failed.
    pub error: Option<String>,
}

impl FailedCount {
    /// "…" until a read answered, the number, or "—" from an older server or when the first read failed.
    pub fn text(&self) -> String {
        match (self.value, &self.error) {
            (Some(Some(n)), _) => n.to_string(),
            (None, None) => "…".to_owned(),
            (Some(None), _) | (None, Some(_)) => "—".to_owned(),
        }
    }

    /// What the count is, and the last read's error.
    pub fn tooltip(&self) -> String {
        let what = match self.value {
            Some(None) => "the server does not report the failed count".to_owned(),
            _ => COUNT_TIP.to_owned(),
        };
        match &self.error {
            Some(error) => format!("{what}; the last read failed: {error}"),
            None => what,
        }
    }
}

/// A fresh read of a message reached by a link.
#[derive(Debug, Clone, PartialEq)]
pub enum Read {
    /// The read is out.
    Loading,
    /// The message, pending, running or finished.
    Found(Arc<MessageStatusView>),
    /// No such message: unknown, or past retention.
    NotFound,
    /// The read failed: the text to show; Retry reads the same id again.
    Failed(String),
}

/// What the pane shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Subject {
    /// An outcome: from its row, or as its place of the history kept it.
    Outcome(Arc<OutcomeView>),
    /// A message reached by a link, by its id, and its read.
    Linked(MessageId, Read),
}

/// The parts of the pane's message, whichever way it came.
#[derive(Debug, Clone, Copy)]
pub struct Details<'a> {
    /// Where it is in its life cycle.
    pub status: MessageState,
    /// The error of a failed message.
    pub error: Option<&'a str>,
    /// When it finished, for a finished message.
    pub finished_at: Option<Timestamp>,
    /// The node holding its lease, for a running message.
    pub node: Option<&'a str>,
    /// Its lease expiry, for a running message.
    pub lease_until: Option<Timestamp>,
    /// The envelope.
    pub message: &'a MessageView,
}

impl Subject {
    /// The message's id.
    pub fn id(&self) -> &MessageId {
        match self {
            Self::Outcome(outcome) => &outcome.id,
            Self::Linked(id, _) => id,
        }
    }

    /// The message, once known.
    pub fn details(&self) -> Option<Details<'_>> {
        match self {
            Self::Outcome(o) => Some(Details {
                status: o.status,
                error: o.error.as_deref(),
                finished_at: Some(o.finished_at),
                node: None,
                lease_until: None,
                message: &o.message,
            }),
            Self::Linked(_, Read::Found(s)) => Some(Details {
                status: s.status,
                error: s.error.as_deref(),
                finished_at: s.finished_at,
                node: s.node.as_deref(),
                lease_until: s.lease_until,
                message: &s.message,
            }),
            Self::Linked(..) => None,
        }
    }
}

/// The messages that record the pane's message as their cause (`causedBy`), one level down, a page at a time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Caused {
    /// Those read so far, in the server's order.
    pub rows: Vec<Arc<MessageStatusView>>,
    /// Whether a page is asked for or failed.
    pub paging: Paging,
    /// True when the server has more ("more…").
    pub more: bool,
    /// True when the message itself is gone: "not found".
    pub gone: bool,
}

impl Caused {
    /// True when the list ended with no rows: "None".
    pub fn none(&self) -> bool {
        self.paging == Paging::Idle && self.rows.is_empty() && !self.more && !self.gone
    }
}

/// What the pane shows of the selected outcome or the linked message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutcomePane {
    /// Grows whenever the pane shows another message, on Refresh and on a context switch.
    pub generation: u64,
    /// The message shown; `None` leaves the pane empty.
    pub subject: Option<Subject>,
    /// Its payload as highlighted JSON, once formatted off the main thread.
    pub payload: Option<Arc<Json>>,
    /// The messages it caused.
    pub caused: Caused,
}

impl OutcomePane {
    /// The message whose "Caused" can be read: an outcome, or a linked message that was found.
    fn parent(&self) -> Option<MessageId> {
        self.subject.as_ref().filter(|subject| subject.details().is_some()).map(|subject| subject.id().clone())
    }
}

/// What a place of the history keeps of the Outcomes pane, restored on a return to it.
#[derive(Debug, Clone, PartialEq)]
pub enum OutcomePlace {
    /// The pane was empty.
    Nothing,
    /// An outcome, as it was read.
    Outcome(Arc<OutcomeView>),
    /// A message reached by a link, read again on a return.
    Linked(MessageId),
}

/// The Outcomes page's reads, publishing an [`OutcomesView`], an [`OutcomePane`] and the [`FailedCount`]. Owned by the
/// session, which hands it the client of the context in use.
pub struct Outcomes {
    shared: Arc<Shared>,
}

/// A read of the failed count that the Overview's refresh makes: it lands unless a later read landed first or the
/// context changed since it was taken.
pub(crate) struct CountTicket {
    shared: Arc<Shared>,
    ticket: u64,
}

impl CountTicket {
    /// Publishes `stats`' failed count, or keeps the last count with the error.
    pub fn land(self, stats: &Part<StatsView>) {
        let mut state = self.shared.lock();
        self.shared.land_count(&mut state, self.ticket, stats.clone().map(|stats| stats.failed));
    }
}

struct Shared {
    handle: Handle,
    limits: Limits,
    list: watch::Sender<OutcomesView>,
    pane: watch::Sender<OutcomePane>,
    count: watch::Sender<FailedCount>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    client: Option<ActorClient>,
    /// True once the list was read for this client; [`Outcomes::show`] reads it only the first time.
    loaded: bool,
    pager: Pager,
    prefix: String,
    status: StatusFilter,
    /// The ids of the rows of this generation: a later row of the same message is dropped.
    seen: HashSet<MessageId>,
    /// The selected row; `None` while the pane shows a message that is not a loaded row.
    selected: Option<MessageId>,
    /// The row of an outcome restored from the history before a page brought it: selected when one does, unless the
    /// pane shows something else by then.
    pending: Option<MessageId>,
    pane_generation: u64,
    format_task: Option<JoinHandle<()>>,
    format_generation: u64,
    link_task: Option<JoinHandle<()>>,
    /// Where the next page of "Caused" starts, and the one read out for it.
    caused_cursor: Cursor,
    caused_task: Option<JoinHandle<()>>,
    /// The last count read started, and the last one that landed or was cut off: an older read is dropped.
    count_asked: u64,
    count_landed: u64,
    count_task: Option<JoinHandle<()>>,
    /// The tasks aborted so far, for tests that await their end.
    #[cfg(test)]
    aborted: Vec<JoinHandle<()>>,
    /// The payloads formatted so far, also those whose result was dropped.
    #[cfg(test)]
    formatted: usize,
}

impl State {
    fn abort(&mut self, task: Option<JoinHandle<()>>) {
        if let Some(task) = task {
            task.abort();
            #[cfg(test)]
            self.aborted.push(task);
        }
    }

    fn supersede_list(&mut self) {
        let [page, debounce] = self.pager.restart();
        self.seen.clear();
        self.abort(page);
        self.abort(debounce);
    }

    fn supersede_pane(&mut self) {
        self.pane_generation += 1;
        self.caused_cursor = Cursor::Start;
        for task in [self.format_task.take(), self.link_task.take(), self.caused_task.take()] {
            self.abort(task);
        }
    }

    /// Drops every count read out.
    fn supersede_count(&mut self) {
        self.count_landed = self.count_asked;
        let task = self.count_task.take();
        self.abort(task);
    }
}

impl Outcomes {
    /// No client yet; reads run on `handle` within `limits.deadline`, and a search waits `limits.debounce`.
    pub fn new(handle: Handle, limits: Limits) -> Self {
        let shared = Shared {
            handle,
            limits,
            list: watch::channel(OutcomesView::default()).0,
            pane: watch::channel(OutcomePane::default()).0,
            count: watch::channel(FailedCount::default()).0,
            state: Mutex::default(),
        };
        Self { shared: Arc::new(shared) }
    }

    /// The list's views, newest last.
    pub fn list(&self) -> watch::Receiver<OutcomesView> {
        self.shared.list.subscribe()
    }

    /// The pane's views, newest last.
    pub fn pane(&self) -> watch::Receiver<OutcomePane> {
        self.shared.pane.subscribe()
    }

    /// The sidebar's failed count, newest last.
    pub fn count(&self) -> watch::Receiver<FailedCount> {
        self.shared.count.subscribe()
    }

    /// A context switch: drops every read, the rows, the pane and the count, which shows "…" again. The list is read
    /// again at the next [`Self::show`].
    pub fn use_client(&self, client: Option<ActorClient>) {
        let mut state = self.shared.lock();
        state.supersede_list();
        state.supersede_count();
        state.client = client;
        state.loaded = false;
        let (generation, prefix, status) = (state.pager.generation(), state.prefix.clone(), state.status);
        self.shared.list.send_replace(OutcomesView { prefix, status, generation, ..OutcomesView::default() });
        self.shared.count.send_replace(FailedCount::default());
        self.shared.show(&mut state, None, None);
    }

    /// Takes a ticket for a read of the failed count made elsewhere: the Overview's refresh.
    pub(crate) fn count_ticket(&self) -> CountTicket {
        let mut state = self.shared.lock();
        state.count_asked += 1;
        CountTicket { shared: self.shared.clone(), ticket: state.count_asked }
    }

    /// The page is shown: reads the list the first time for this client.
    pub fn show(&self) {
        let mut state = self.shared.lock();
        if !state.loaded && state.client.is_some() {
            self.shared.reload(&mut state, false);
        }
    }

    /// Re-reads the loaded range without clearing the view or changing the list generation; [`Done`] when it ends.
    pub fn soft_refresh(&self) -> Done {
        self.shared.request()
    }

    /// Cancels a soft refresh when this page is left.
    pub fn cancel_soft_refresh(&self) {
        self.shared.lock().pager.cancel_refresh();
    }

    /// Refresh: reads the list again from the top and the failed count. An outcome in the pane is cleared, as its row
    /// may be gone; a linked message is read again.
    pub fn refresh(&self) {
        let mut state = self.shared.lock();
        let Some(client) = state.client.clone() else { return };
        let linked = match &self.shared.pane.borrow().subject {
            Some(Subject::Linked(id, _)) => Some(id.clone()),
            _ => None,
        };
        let subject = linked.map(|id| Subject::Linked(id, Read::Loading));
        self.shared.show(&mut state, subject, None);
        self.shared.reload(&mut state, false);
        state.count_asked += 1;
        let (ticket, shared, deadline) = (state.count_asked, self.shared.clone(), self.shared.limits.deadline);
        let task = self.shared.handle.spawn(async move {
            let answer = within(Instant::now() + deadline, client.stats("")).await;
            let mut state = shared.lock();
            if state.count_asked == ticket {
                state.count_task = None;
            }
            shared.land_count(&mut state, ticket, answer.map(|stats| stats.failed));
        });
        let before = state.count_task.replace(task);
        state.abort(before);
    }

    /// Resolves once a Refresh has ended: the first page, the failed count, and a linked message and the messages it
    /// caused have answered or failed.
    pub fn reloaded(&self) -> impl Future<Output = ()> + Send + 'static {
        let shared = self.shared.clone();
        let (mut list, mut pane, mut count) = (self.list(), self.pane(), self.count());
        async move {
            loop {
                let busy = list.borrow_and_update().paging == Paging::Loading
                    || {
                        let pane = pane.borrow_and_update();
                        matches!(pane.subject, Some(Subject::Linked(_, Read::Loading)))
                            || pane.caused.paging == Paging::Loading
                    }
                    || {
                        count.borrow_and_update();
                        shared.lock().count_task.is_some()
                    };
                if !busy {
                    return;
                }
                let changed = tokio::select! {
                    changed = list.changed() => changed,
                    changed = pane.changed() => changed,
                    changed = count.changed() => changed,
                };
                if changed.is_err() {
                    return;
                }
            }
        }
    }

    /// A key press in the search field: the rows go at once, and the new prefix is read once the field has been
    /// still for `limits.debounce`. A selected outcome of an actor outside the prefix is unselected, and one that the
    /// new pages do not hold once they are all read.
    pub fn search(&self, prefix: &str) {
        let mut state = self.shared.lock();
        state.prefix = prefix.to_owned();
        let outside = state.selected.is_some()
            && matches!(&self.shared.pane.borrow().subject,
                Some(Subject::Outcome(o)) if !o.actor.as_str().starts_with(prefix));
        if outside {
            self.shared.show(&mut state, None, None);
        }
        if state.client.is_none() {
            self.shared.list.send_modify(|list| list.prefix = prefix.to_owned());
            return;
        }
        self.shared.reload(&mut state, true);
    }

    /// The status switch: a new generation that resets the cursor, the seen ids, a pending search, the selection and
    /// the pane's work together, and reads the first page of `status` at once.
    pub fn set_status(&self, status: StatusFilter) {
        let mut state = self.shared.lock();
        if state.status == status {
            return;
        }
        state.status = status;
        self.shared.show(&mut state, None, None);
        if state.client.is_none() {
            state.supersede_list();
            self.shared.list.send_modify(|list| {
                list.status = status;
                list.rows.clear();
            });
            return;
        }
        self.shared.reload(&mut state, false);
    }

    /// Asks for the next page, unless the last page is read, a page is out or the last one failed.
    pub fn more(&self) {
        let mut state = self.shared.lock();
        if self.shared.list.borrow().paging == Paging::Idle {
            self.shared.request_page(&mut state);
        }
    }

    /// Asks again for the page that failed, from the cursor it kept.
    pub fn retry(&self) {
        let mut state = self.shared.lock();
        if matches!(self.shared.list.borrow().paging, Paging::Failed(_)) {
            self.shared.request_page(&mut state);
        }
    }

    /// Selects the outcome `id` of the rows and fills the pane from its row, with no read of it; its payload is
    /// formatted now and the messages it caused are read. `None`, or an id not among the rows, empties the pane.
    pub fn select(&self, id: Option<&MessageId>) {
        let mut state = self.shared.lock();
        let row = id.and_then(|id| {
            let list = self.shared.list.borrow();
            list.index(id).map(|index| list.rows[index].clone())
        });
        let selected = row.as_ref().map(|row| row.id.clone());
        self.shared.show(&mut state, row.map(Subject::Outcome), selected);
    }

    /// The actor the pane shows, if any.
    pub fn pane_actor(&self) -> Option<ActorAddress> {
        self.shared.pane.borrow().subject.as_ref().and_then(Subject::details).map(|details| details.message.to.clone())
    }

    /// The selected row's message id.
    pub fn selected(&self) -> Option<MessageId> {
        self.shared.lock().selected.clone()
    }

    /// What the pane shows, for the history to restore later.
    pub fn source(&self) -> OutcomePlace {
        match &self.shared.pane.borrow().subject {
            None => OutcomePlace::Nothing,
            Some(Subject::Outcome(outcome)) => OutcomePlace::Outcome(outcome.clone()),
            Some(Subject::Linked(id, _)) => OutcomePlace::Linked(id.clone()),
        }
    }

    /// Restores a place of the history: an outcome as it was read, with its row selected when it is loaded or once a
    /// page brings it; or a linked message, read again.
    pub fn restore(&self, place: OutcomePlace) {
        let mut state = self.shared.lock();
        match place {
            OutcomePlace::Nothing => self.shared.show(&mut state, None, None),
            OutcomePlace::Outcome(outcome) => {
                let id = outcome.id.clone();
                let loaded = self.shared.list.borrow().index(&id).is_some();
                self.shared.show(&mut state, Some(Subject::Outcome(outcome)), loaded.then(|| id.clone()));
                if !loaded {
                    state.pending = Some(id);
                }
            }
            OutcomePlace::Linked(id) => self.shared.show(&mut state, Some(Subject::Linked(id, Read::Loading)), None),
        }
    }

    /// Reads a linked message again after its read failed.
    pub fn retry_link(&self) {
        let mut state = self.shared.lock();
        let failed = match &self.shared.pane.borrow().subject {
            Some(Subject::Linked(id, Read::Failed(_))) => Some(id.clone()),
            _ => None,
        };
        if let Some(id) = failed {
            self.shared.show(&mut state, Some(Subject::Linked(id, Read::Loading)), None);
        }
    }

    /// "more…" of Caused: asks for its next page, unless one is out or failed or the list ended.
    pub fn more_caused(&self) {
        let mut state = self.shared.lock();
        let parent = {
            let pane = self.shared.pane.borrow();
            pane.parent().filter(|_| pane.caused.paging == Paging::Idle && pane.caused.more)
        };
        if let Some(parent) = parent {
            self.shared.request_caused(&mut state, parent);
        }
    }

    /// Asks again for the page of Caused that failed, from the cursor it kept.
    pub fn retry_caused(&self) {
        let mut state = self.shared.lock();
        let parent = {
            let pane = self.shared.pane.borrow();
            pane.parent().filter(|_| matches!(pane.caused.paging, Paging::Failed(_)))
        };
        if let Some(parent) = parent {
            self.shared.request_caused(&mut state, parent);
        }
    }

    /// The window closes: every read and formatting is dropped.
    pub fn stop(&self) {
        let mut state = self.shared.lock();
        state.supersede_list();
        state.supersede_pane();
        state.supersede_count();
    }

    /// The tasks aborted so far; each ends once it has stopped.
    #[cfg(test)]
    pub fn take_aborted(&self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.shared.lock().aborted)
    }

    /// How many payloads were formatted so far, also those whose result was dropped.
    #[cfg(test)]
    pub fn formatted(&self) -> usize {
        self.shared.lock().formatted
    }
}

impl Drop for Outcomes {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Paged for Shared {
    type State = State;

    fn handle(&self) -> &Handle {
        &self.handle
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("inspector outcomes")
    }

    fn pager(state: &mut State) -> &mut Pager {
        &mut state.pager
    }
}

impl Shared {
    /// Replaces the rows under a new generation and reads the first page: now, or after the debounce.
    fn reload(self: &Arc<Self>, state: &mut State, debounce: bool) {
        state.supersede_list();
        state.loaded = true;
        let (generation, prefix, status) = (state.pager.generation(), state.prefix.clone(), state.status);
        self.list.send_replace(OutcomesView {
            prefix,
            status,
            generation,
            paging: Paging::Loading,
            more: true,
            ..OutcomesView::default()
        });
        if debounce {
            Pager::debounce(self, state, self.limits, |shared, state| shared.request_page(state));
        } else {
            self.request_page(state);
        }
    }

    fn request_page(self: &Arc<Self>, state: &mut State) {
        let Some(client) = state.client.clone() else { return };
        let query = (state.prefix.clone(), state.status.status());
        let read = move |cursor| Self::read_page(client, query, cursor);
        if Pager::request(self, state, self.limits, read, Self::land_page) {
            self.list.send_modify(|list| list.paging = Paging::Loading);
        }
    }

    /// Appends the outcomes of a page whose message was not seen before in this generation. A page that adds none
    /// asks for the next at once while the server has more: the rows the view sees did not change, so it would not
    /// ask again.
    fn land_page(self: &Arc<Self>, state: &mut State, landed: Landed<OutcomeView>) {
        let (items, more) = match landed.page {
            Part::Ready(page) => page,
            Part::Failed(error) => {
                self.list.send_modify(|list| list.paging = Paging::Failed(error));
                return self.resume(state);
            }
        };
        let fresh: Vec<_> = items.into_iter().filter(|o| state.seen.insert(o.id.clone())).map(Arc::new).collect();
        let added = !fresh.is_empty();
        // The restored outcome's row arrived: it is selected, and the pane keeps the outcome as it was read.
        if let Some(id) = state.pending.take_if(|id| fresh.iter().any(|row| &row.id == id)) {
            state.selected = Some(id);
        } else if !more {
            state.pending = None;
        }
        self.list.send_modify(|list| {
            list.rows.extend(fresh);
            list.paging = Paging::Idle;
            list.more = more;
        });
        // Every page is read: a selected outcome that is not among the rows is gone from this list, and its pane.
        let absent = !more && state.selected.as_ref().is_some_and(|id| self.list.borrow().index(id).is_none());
        if absent {
            self.show(state, None, None);
        }
        if !added && more {
            self.request_page(state);
        }
        self.resume(state);
    }

    /// Shows `subject` under a new pane generation, with `selected` the row selected: an outcome's payload is
    /// formatted and its Caused read now, a linked message is read first. Work of the message before is dropped.
    fn show(self: &Arc<Self>, state: &mut State, subject: Option<Subject>, selected: Option<MessageId>) {
        state.supersede_pane();
        state.selected = selected;
        state.pending = None;
        let generation = state.pane_generation;
        // An outcome's Caused is read at once: it shows as loading from the first view, never as "None".
        let reads = matches!(subject, Some(Subject::Outcome(_))) && state.client.is_some();
        let caused = Caused { paging: if reads { Paging::Loading } else { Paging::Idle }, ..Caused::default() };
        self.pane.send_replace(OutcomePane { generation, subject: subject.clone(), caused, ..OutcomePane::default() });
        match subject {
            Some(Subject::Outcome(outcome)) => {
                let id = outcome.id.clone();
                self.format(state, outcome, |o| &o.message.payload);
                self.request_caused(state, id);
            }
            Some(Subject::Linked(id, Read::Loading)) => self.read_link(state, id),
            _ => {}
        }
    }

    /// Formats the payload that `payload` finds in `source` on a blocking thread; the result is published only while
    /// the pane's generation is the same, so one that finishes after the pane moved on is dropped.
    fn format<T: Send + Sync + 'static>(
        self: &Arc<Self>,
        state: &mut State,
        source: Arc<T>,
        payload: fn(&T) -> &Value,
    ) {
        let previous = state.format_task.take();
        state.abort(previous);
        state.format_generation += 1;
        let (generation, format, shared) = (state.pane_generation, state.format_generation, self.clone());
        state.format_task = Some(self.handle.spawn_blocking(move || {
            let json = Arc::new(json::format(payload(&source)));
            let mut state = shared.lock();
            #[cfg(test)]
            {
                state.formatted += 1;
            }
            if state.pane_generation == generation && state.format_generation == format {
                state.format_task = None;
                shared.pane.send_modify(|pane| pane.payload = Some(json));
            }
        }));
    }

    /// Reads the linked message `id` fresh; once found, formats its payload and reads its Caused.
    fn read_link(self: &Arc<Self>, state: &mut State, id: MessageId) {
        let Some(client) = state.client.clone() else {
            let failed = Subject::Linked(id, Read::Failed("No server is configured.".into()));
            return self.pane.send_modify(|pane| pane.subject = Some(failed));
        };
        let (generation, shared, deadline) = (state.pane_generation, self.clone(), self.limits.deadline);
        state.link_task = Some(self.handle.spawn(async move {
            let answer = within(Instant::now() + deadline, client.status(&id)).await;
            let mut state = shared.lock();
            if state.pane_generation != generation {
                return;
            }
            state.link_task = None;
            let read = match answer {
                Part::Ready(Some(status)) => Read::Found(Arc::new(status)),
                Part::Ready(None) => Read::NotFound,
                Part::Failed(error) => Read::Failed(error),
            };
            let found = match &read {
                Read::Found(status) => Some(status.clone()),
                _ => None,
            };
            // A found message's Caused is read at once: the view that shows it shows Caused loading, never "None".
            let reads = found.is_some();
            shared.pane.send_modify(|pane| {
                pane.subject = Some(Subject::Linked(id.clone(), read));
                if reads {
                    pane.caused.paging = Paging::Loading;
                }
            });
            if let Some(status) = found {
                shared.format(&mut state, status, |s| &s.message.payload);
                shared.request_caused(&mut state, id);
            }
        }));
    }

    /// Reads the page of Caused at its cursor, unless one is out or the list ended: one request per pane. A failed
    /// page keeps the cursor, so asking again is Retry; a page with no rows but a next cursor is followed at once.
    fn request_caused(self: &Arc<Self>, state: &mut State, parent: MessageId) {
        let Some(client) = state.client.clone() else { return };
        let cursor = match &state.caused_cursor {
            _ if state.caused_task.is_some() => return,
            Cursor::End => return,
            Cursor::Start => None,
            Cursor::Next(cursor) => Some(cursor.clone()),
        };
        let (generation, shared, deadline) = (state.pane_generation, self.clone(), self.limits.deadline);
        state.caused_task = Some(self.handle.spawn(async move {
            let call = client.caused_by(&parent, cursor.as_deref(), Some(CAUSED_PAGE));
            let answer = within(Instant::now() + deadline, call).await;
            let mut state = shared.lock();
            if state.pane_generation != generation {
                return;
            }
            state.caused_task = None;
            let follow = shared.land_caused(&mut state, answer);
            if follow {
                shared.request_caused(&mut state, parent);
            }
        }));
        self.pane.send_modify(|pane| pane.caused.paging = Paging::Loading);
    }

    fn land_caused(&self, state: &mut State, answer: Part<Option<cicada::api::PageView<MessageStatusView>>>) -> bool {
        match answer {
            Part::Ready(None) => {
                state.caused_cursor = Cursor::End;
                self.pane.send_modify(|pane| {
                    pane.caused = Caused { gone: true, ..Caused::default() };
                });
                false
            }
            Part::Ready(Some(page)) => {
                state.caused_cursor = page.next.map_or(Cursor::End, Cursor::Next);
                let more = state.caused_cursor != Cursor::End;
                let empty = page.items.is_empty();
                self.pane.send_modify(|pane| {
                    pane.caused.rows.extend(page.items.into_iter().map(Arc::new));
                    pane.caused.more = more;
                    pane.caused.paging = if empty && more { Paging::Loading } else { Paging::Idle };
                });
                empty && more
            }
            Part::Failed(error) => {
                self.pane.send_modify(|pane| pane.caused.paging = Paging::Failed(error));
                false
            }
        }
    }

    fn mark_pane_failure(&self, state: &mut State, error: String) {
        let mark = self.list.borrow().stale.clone();
        let as_of = crate::refresh::as_of(mark.as_ref(), state.pager.read_at());
        self.list.send_modify(|list| {
            let mark = list.stale.get_or_insert_with(|| Stale::failed(as_of, error.clone()));
            match &mut mark.error {
                Some(before) if !before.contains(&error) => {
                    before.push('\n');
                    before.push_str(&error);
                }
                None => mark.error = Some(error),
                _ => {}
            }
        });
    }

    /// Publishes a count read's answer unless a later read landed first or the context changed since it started; a
    /// failed read keeps the count with its error.
    fn land_count(&self, state: &mut State, ticket: u64, failed: Part<Option<u64>>) {
        if ticket <= state.count_landed {
            return;
        }
        state.count_landed = ticket;
        self.count.send_modify(|count| match failed {
            Part::Ready(value) => *count = FailedCount { value: Some(value), error: None },
            Part::Failed(error) => count.error = Some(error),
        });
    }
}

/// The first line of an error, for its table cell.
pub fn first_line(error: &str) -> &str {
    error.lines().next().unwrap_or("")
}

/// A message state's name, as on the wire.
pub fn state_text(state: MessageState) -> &'static str {
    match state {
        MessageState::Pending => "pending",
        MessageState::Running => "running",
        MessageState::Done => "done",
        MessageState::Failed => "failed",
        MessageState::Cancelled => "cancelled",
        MessageState::Purged => "purged",
    }
}

/// The retry policy in words.
pub fn retry_text(policy: Option<&RetryPolicy>) -> String {
    let Some(policy) = policy else { return "No retry policy".to_owned() };
    let attempts = policy.max_attempts();
    match policy.backoff() {
        Backoff::Exponential { initial, factor, max } => {
            format!("Up to {attempts} attempts, backoff from {initial:?} ×{factor} to at most {max:?}")
        }
        Backoff::Fixed { delay } => format!("Up to {attempts} attempts, {delay:?} apart"),
    }
}

impl SoftPaged for Shared {
    type Row = OutcomeView;
    type Key = MessageId;
    // The store's documented order of outcomes, `(finished_at, message_id)`.
    type Sort = (Timestamp, MessageId);
    type Query = (String, Option<OutcomeStatus>);

    fn key(row: &Self::Row) -> Self::Key {
        row.id.clone()
    }

    fn sort(row: &Self::Row) -> Self::Sort {
        (row.finished_at, row.id.clone())
    }

    fn rows(&self, _state: &State) -> Vec<Arc<Self::Row>> {
        self.list.borrow().rows.clone()
    }

    fn paging(&self) -> Paging {
        self.list.borrow().paging.clone()
    }

    fn limits(&self) -> Limits {
        self.limits
    }

    fn stale(&self) -> Option<Stale> {
        self.list.borrow().stale.clone()
    }

    fn query(state: &State) -> Option<(ActorClient, Self::Query)> {
        Some((state.client.clone().filter(|_| state.loaded)?, (state.prefix.clone(), state.status.status())))
    }

    async fn read_page(
        client: ActorClient,
        query: Self::Query,
        cursor: Option<String>,
    ) -> Result<cicada::api::PageView<Self::Row>, cicada_client::ClientError> {
        client.outcomes(query.1, &query.0, None, cursor.as_deref(), Some(PAGE)).await
    }

    fn land_refresh(
        self: &Arc<Self>,
        state: &mut State,
        rows: Option<Vec<Arc<Self::Row>>>,
        more: bool,
        stale: Option<Stale>,
    ) {
        let Some(rows) = rows else {
            return self.list.send_modify(|list| list.stale = stale);
        };
        state.seen = rows.iter().map(|row| row.id.clone()).collect();
        self.list.send_modify(|list| {
            list.rows = rows;
            list.more = more;
            list.stale = stale;
        });
        let selected = state.pending.take_if(|id| self.list.borrow().index(id).is_some()).or(state.selected.clone());
        if let Some(id) = selected {
            let current = self.list.borrow().rows.iter().find(|row| row.id == id).cloned();
            match current {
                Some(row) => {
                    state.selected = Some(id);
                    let same =
                        matches!(&self.pane.borrow().subject, Some(Subject::Outcome(before)) if **before == *row);
                    if !same {
                        self.pane.send_modify(|pane| pane.subject = Some(Subject::Outcome(row.clone())));
                        self.format(state, row, |o| &o.message.payload);
                    }
                }
                None => self.show(state, None, None),
            }
        }
        if !more {
            state.pending = None;
        }
    }

    async fn refresh_pane(self: &Arc<Self>, ticket: u64, budget: Budget) {
        let (client, id, generation, had_details) = {
            let state = self.lock();
            if !state.pager.refresh.current(ticket) || state.link_task.is_some() {
                return;
            }
            let pane = self.pane.borrow();
            let Some(Subject::Linked(id, read)) = &pane.subject else { return };
            let Some(client) = state.client.clone() else { return };
            (client, id.clone(), state.pane_generation, matches!(read, Read::Found(_)))
        };
        let answer = budget.read(client.status(&id)).await;
        // A previously unresolved link has no Caused page yet. Keep the old pane until its first page has answered.
        let caused = if !had_details && matches!(answer, Part::Ready(Some(_))) {
            let mut cursor = None;
            loop {
                let answer = budget.read(client.caused_by(&id, cursor.as_deref(), Some(CAUSED_PAGE))).await;
                match answer {
                    Part::Ready(Some(page)) if page.items.is_empty() && page.next.is_some() => cursor = page.next,
                    _ => break Some((cursor, answer)),
                }
            }
        } else {
            None
        };
        let mut state = self.lock();
        if !state.pager.refresh.current(ticket) || state.pane_generation != generation {
            return;
        }
        match answer {
            Part::Ready(Some(status)) => {
                let status = Arc::new(status);
                let payload_changed = self
                    .pane
                    .borrow()
                    .subject
                    .as_ref()
                    .and_then(Subject::details)
                    .is_none_or(|details| details.message.payload != status.message.payload);
                if let Some((cursor, caused)) = caused {
                    if let Part::Failed(error) = &caused {
                        self.mark_pane_failure(&mut state, error.clone());
                    }
                    state.caused_cursor = cursor.map_or(Cursor::Start, Cursor::Next);
                    self.land_caused(&mut state, caused);
                }
                self.pane.send_modify(|pane| pane.subject = Some(Subject::Linked(id, Read::Found(status.clone()))));
                if payload_changed {
                    self.format(&mut state, status, |s| &s.message.payload);
                }
            }
            Part::Ready(None) => {
                state.supersede_pane();
                self.pane.send_replace(OutcomePane {
                    generation: state.pane_generation,
                    subject: Some(Subject::Linked(id, Read::NotFound)),
                    ..OutcomePane::default()
                });
            }
            Part::Failed(error) => self.mark_pane_failure(&mut state, error),
        }
    }

    fn page(self: &Arc<Self>, state: &mut State) {
        self.request_page(state);
    }
}
