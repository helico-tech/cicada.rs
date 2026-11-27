//! The Messages page: the pending messages under a prefix, read a page at a time through the root pager, each message
//! once per generation; and the pane of the selected one, filled from its row with its payload formatted only then.
//! The list's generation grows at every key press in the search field, on Refresh and on a context switch; the pane's
//! at every selection, on Refresh and on a context switch.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use cicada::api::MessageView;
use cicada::store::MessageId;
use cicada_client::ActorClient;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::json::{self, Json};
use crate::model::{Limits, Part};
use crate::pager::{Landed, NEAR_END, PAGE, Paged, Pager, Paging};

/// What the Messages table shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessagesView {
    /// The prefix of the target addresses the rows are under.
    pub prefix: String,
    /// Grows whenever the rows are replaced rather than appended to.
    pub generation: u64,
    /// The messages read so far, each once, in the server's order; shared with the pane and with later views.
    pub rows: Vec<Arc<MessageView>>,
    /// Whether a page is asked for or failed.
    pub paging: Paging,
    /// True when the server has rows past these.
    pub more: bool,
}

impl MessagesView {
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

/// What the pane shows of the selected message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessagePane {
    /// Grows at every selection, Refresh and context switch.
    pub generation: u64,
    /// The selected message, from its row; `None` leaves the pane empty.
    pub message: Option<Arc<MessageView>>,
    /// Its payload as highlighted JSON, once formatted off the main thread.
    pub payload: Option<Arc<Json>>,
}

/// The Messages page's reads, publishing a [`MessagesView`] and a [`MessagePane`]. Owned by the session, which hands
/// it the client of the context in use.
pub struct Messages {
    shared: Arc<Shared>,
}

struct Shared {
    handle: Handle,
    limits: Limits,
    list: watch::Sender<MessagesView>,
    pane: watch::Sender<MessagePane>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    client: Option<ActorClient>,
    /// True once the list was read for this client; [`Messages::show`] reads it only the first time.
    loaded: bool,
    pager: Pager,
    prefix: String,
    /// The ids of the rows of this generation: a later row of the same message is dropped.
    seen: HashSet<MessageId>,
    pane_generation: u64,
    selected: Option<MessageId>,
    format_task: Option<JoinHandle<()>>,
    /// The tasks aborted so far, for tests that await their end.
    #[cfg(test)]
    aborted: Vec<JoinHandle<()>>,
    /// The payloads formatted so far.
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
        let task = self.format_task.take();
        self.abort(task);
    }
}

impl Messages {
    /// No client yet; reads run on `handle` within `limits.deadline`, and a search waits `limits.debounce`.
    pub fn new(handle: Handle, limits: Limits) -> Self {
        let shared = Shared {
            handle,
            limits,
            list: watch::channel(MessagesView::default()).0,
            pane: watch::channel(MessagePane::default()).0,
            state: Mutex::default(),
        };
        Self { shared: Arc::new(shared) }
    }

    /// The list's views, newest last.
    pub fn list(&self) -> watch::Receiver<MessagesView> {
        self.shared.list.subscribe()
    }

    /// The pane's views, newest last.
    pub fn pane(&self) -> watch::Receiver<MessagePane> {
        self.shared.pane.subscribe()
    }

    /// A context switch: drops every read, the rows and the selection. The list is read again at the next
    /// [`Self::show`].
    pub fn use_client(&self, client: Option<ActorClient>) {
        let mut state = self.shared.lock();
        state.supersede_list();
        state.client = client;
        state.loaded = false;
        let (generation, prefix) = (state.pager.generation(), state.prefix.clone());
        self.shared.list.send_replace(MessagesView { prefix, generation, ..MessagesView::default() });
        self.shared.select(&mut state, None);
    }

    /// The page is shown: reads the list the first time for this client.
    pub fn show(&self) {
        let mut state = self.shared.lock();
        if !state.loaded && state.client.is_some() {
            self.shared.reload(&mut state, false);
        }
    }

    /// Refresh: reads the list again from the top under its prefix; the selection is cleared, as its row may be gone.
    pub fn refresh(&self) {
        let mut state = self.shared.lock();
        if state.client.is_none() {
            return;
        }
        self.shared.select(&mut state, None);
        self.shared.reload(&mut state, false);
    }

    /// A key press in the search field: the rows go at once, and the new prefix is read once the field has been
    /// still for `limits.debounce`. A selected message to an address outside the prefix is unselected.
    pub fn search(&self, prefix: &str) {
        let mut state = self.shared.lock();
        state.prefix = prefix.to_owned();
        let outside = self.shared.pane.borrow().message.as_ref().is_some_and(|m| !m.to.as_str().starts_with(prefix));
        if outside {
            self.shared.select(&mut state, None);
        }
        if state.client.is_none() {
            self.shared.list.send_modify(|list| list.prefix = prefix.to_owned());
            return;
        }
        self.shared.reload(&mut state, true);
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

    /// Selects the message `id` of the rows and fills the pane from its row; its payload is formatted now, and a
    /// formatting of an earlier selection is dropped. `None`, or an id not among the rows, empties the pane.
    pub fn select(&self, id: Option<&MessageId>) {
        let mut state = self.shared.lock();
        self.shared.select(&mut state, id);
    }

    /// The selected message's id.
    pub fn selected(&self) -> Option<MessageId> {
        self.shared.lock().selected.clone()
    }

    /// The window closes: every read and formatting is dropped.
    pub fn stop(&self) {
        let mut state = self.shared.lock();
        state.supersede_list();
        state.supersede_pane();
    }

    /// The tasks aborted so far; each ends once it has stopped.
    #[cfg(test)]
    pub fn take_aborted(&self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.shared.lock().aborted)
    }

    /// How many payloads were formatted so far.
    #[cfg(test)]
    pub fn formatted(&self) -> usize {
        self.shared.lock().formatted
    }
}

impl Drop for Messages {
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
        self.state.lock().expect("inspector messages")
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
        let (generation, prefix) = (state.pager.generation(), state.prefix.clone());
        self.list.send_replace(MessagesView { prefix, generation, paging: Paging::Loading, more: true, rows: vec![] });
        if debounce {
            Pager::debounce(self, state, self.limits, |shared, state| shared.request_page(state));
        } else {
            self.request_page(state);
        }
    }

    fn request_page(self: &Arc<Self>, state: &mut State) {
        let Some(client) = state.client.clone() else { return };
        let prefix = state.prefix.clone();
        let read = move |cursor: Option<String>| async move {
            client.list_pending(&prefix, cursor.as_deref(), Some(PAGE)).await
        };
        if Pager::request(self, state, self.limits, read, Self::land_page) {
            self.list.send_modify(|list| list.paging = Paging::Loading);
        }
    }

    /// Appends the messages of a page not seen before in this generation. A page that adds none asks for the next
    /// at once while the server has more: the rows the view sees did not change, so it would not ask again.
    fn land_page(self: &Arc<Self>, state: &mut State, landed: Landed<MessageView>) {
        let (items, more) = match landed.page {
            Part::Ready(page) => page,
            Part::Failed(error) => return self.list.send_modify(|list| list.paging = Paging::Failed(error)),
        };
        let fresh: Vec<_> = items.into_iter().filter(|m| state.seen.insert(m.id.clone())).map(Arc::new).collect();
        let added = !fresh.is_empty();
        self.list.send_modify(|list| {
            list.rows.extend(fresh);
            list.paging = Paging::Idle;
            list.more = more;
        });
        if !added && more {
            self.request_page(state);
        }
    }

    /// Selects a row under a new pane generation and formats its payload, published only while it is still selected.
    fn select(self: &Arc<Self>, state: &mut State, id: Option<&MessageId>) {
        state.supersede_pane();
        let generation = state.pane_generation;
        let row = id.and_then(|id| {
            let list = self.list.borrow();
            list.index(id).map(|index| list.rows[index].clone())
        });
        state.selected = row.as_ref().map(|row| row.id.clone());
        self.pane.send_replace(MessagePane { generation, message: row.clone(), payload: None });
        let Some(row) = row else { return };
        let shared = self.clone();
        state.format_task = Some(self.handle.spawn(async move {
            let payload = Arc::new(json::format(&row.payload));
            let mut state = shared.lock();
            #[cfg(test)]
            {
                state.formatted += 1;
            }
            if state.pane_generation == generation {
                state.format_task = None;
                shared.pane.send_modify(|pane| pane.payload = Some(payload));
            }
        }));
    }
}
