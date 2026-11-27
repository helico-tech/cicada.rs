//! The Actors page: the paged list of actors under a prefix with its sample of running turns, and the pane's reads of
//! the selected actor. Every read carries the generation it was asked under and is dropped when that no longer
//! matches: the list's generation grows at every key press in the search field, on Refresh and on a context switch;
//! the pane's at every selection, on Refresh and on a context switch.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use cicada::ActorAddress;
use cicada::api::{ActorView, MessageState, RunningTurnView, WatchView};
use cicada::store::{MessageId, NodeId};
use cicada::time::Timestamp;
use cicada::watch::WatchTarget;
use cicada_client::ActorClient;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout_at};

use crate::json::{self, Json};
use crate::model::{Limits, Part, TIMED_OUT, within};

/// The rows of one page of the list.
pub const PAGE: usize = 100;
/// The running turns read once per reload of the list to mark rows running.
pub const SAMPLE: usize = 200;
/// The view asks for the next page when the last visible row is this close to the end.
pub const NEAR_END: usize = 20;
/// The mailbox rows the pane reads.
pub const MAILBOX_ROWS: usize = 50;
/// The watches of each kind the pane reads.
pub const WATCH_ROWS: usize = 50;

/// An actor of the list: the metadata of its row. The state of the listing is dropped once the row is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorRow {
    /// Its address.
    pub address: ActorAddress,
    /// Its type's name.
    pub type_name: String,
    /// The commits that wrote its state.
    pub version: i64,
    /// When it was created.
    pub created_at: Timestamp,
    /// True while its mailbox is paused.
    pub paused: bool,
}

impl From<ActorView> for ActorRow {
    fn from(view: ActorView) -> Self {
        Self {
            address: view.address,
            type_name: view.type_name,
            version: view.version,
            created_at: view.created_at,
            paused: view.paused,
        }
    }
}

/// A row's path from the root list, including its own address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ActorPath(Vec<ActorAddress>);

impl ActorPath {
    /// The occurrence of an actor in the root list.
    pub fn root(address: ActorAddress) -> Self {
        Self(vec![address])
    }

    /// The occurrence of `address` in this row's children.
    pub fn child(&self, address: ActorAddress) -> Self {
        let mut path = self.clone();
        path.0.push(address);
        path
    }

    /// The actor at this row.
    pub fn address(&self) -> &ActorAddress {
        self.0.last().expect("a path starts with a root actor")
    }

    /// True for the occurrence in the root list.
    pub fn is_root(&self) -> bool {
        self.0.len() == 1
    }

    /// The parent row, or `None` for a root row.
    pub fn parent(&self) -> Option<Self> {
        (!self.is_root()).then(|| Self(self.0[..self.0.len() - 1].to_vec()))
    }
}

/// The state of the list's paging.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Paging {
    /// No page is asked for.
    #[default]
    Idle,
    /// A page is asked for, or the first root page waits for the search field to settle.
    Loading,
    /// The last page failed and can be asked again with Retry.
    Failed(String),
}

/// The loaded children and paging state of one occurrence of a parent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Children {
    /// Child addresses loaded so far, in address order.
    pub rows: Vec<ActorAddress>,
    /// Whether a page is asked for or failed.
    pub paging: Paging,
    /// True when the server says the parent has gone.
    pub not_found: bool,
    /// True when the server has more children after those read so far.
    pub more: bool,
}

/// Which actors run a turn, from one read of the first [`SAMPLE`] running turns under the list's prefix.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Running {
    /// Not read yet.
    #[default]
    Loading,
    /// The addresses of the running turns read; `partial` when more turns run than were read.
    Sample {
        /// The actors with a running turn among those read.
        actors: BTreeSet<ActorAddress>,
        /// True when the server has more running turns than [`SAMPLE`].
        partial: bool,
    },
    /// The read failed: no row is marked running.
    Unknown(String),
}

/// What the Actors table shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListView {
    /// The prefix the rows are under.
    pub prefix: String,
    /// Grows whenever the rows are replaced rather than appended to.
    pub generation: u64,
    /// The rows read so far, in address order.
    pub rows: Vec<ActorRow>,
    /// Whether a page is asked for or failed.
    pub paging: Paging,
    /// True when the server has rows past these.
    pub more: bool,
    /// The running turns' sample.
    pub running: Running,
    /// Children read at each parent row, including rows since closed.
    pub children: BTreeMap<ActorPath, Children>,
    /// Open rows to restore as they are loaded, including those still awaiting their page.
    pub open: BTreeSet<ActorPath>,
}

/// The status column of a row: both hold together when they do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RowStatus {
    /// A turn of it is in the sample of running turns.
    pub running: bool,
    /// Its mailbox is paused.
    pub paused: bool,
}

impl RowStatus {
    /// `running`, `paused`, `running · paused`, or empty for an idle actor.
    pub fn text(self) -> &'static str {
        match (self.running, self.paused) {
            (true, true) => "running · paused",
            (true, false) => "running",
            (false, true) => "paused",
            (false, false) => "",
        }
    }
}

impl ListView {
    /// The status of `row`: running from the sample, paused from the row.
    pub fn status(&self, row: &ActorRow) -> RowStatus {
        let running = matches!(&self.running, Running::Sample { actors, .. } if actors.contains(&row.address));
        RowStatus { running, paused: row.paused }
    }

    /// The running status of a child, with `—` outside the sample's prefix.
    pub fn child_status(&self, child: &ActorAddress) -> &'static str {
        if !child.as_str().starts_with(&self.prefix) {
            "—"
        } else if matches!(&self.running, Running::Sample { actors, .. } if actors.contains(child)) {
            "running"
        } else {
            ""
        }
    }

    /// The index of this address in the root list.
    pub fn root_index(&self, address: &ActorAddress) -> Option<usize> {
        self.rows.binary_search_by(|row| row.address.cmp(address)).ok()
    }

    fn contains(&self, path: &ActorPath) -> bool {
        match path.parent() {
            Some(parent) => {
                self.children.get(&parent).is_some_and(|page| page.rows.binary_search(path.address()).is_ok())
            }
            None => self.root_index(path.address()).is_some(),
        }
    }

    /// The footer under the table: what the running status covers, when it is not every running turn.
    pub fn footer(&self) -> Option<String> {
        match &self.running {
            Running::Sample { partial: true, .. } => {
                Some(format!("The running status covers the first {SAMPLE} running turns."))
            }
            Running::Unknown(error) => Some(format!("The running status is unknown: {error}")),
            Running::Loading | Running::Sample { partial: false, .. } => None,
        }
    }

    /// True when the view should ask for the next page with `last` the last visible root index: it is within
    /// [`NEAR_END`] of the end, the server has more, and no page is asked for or failed.
    pub fn wants_more(&self, last: usize) -> bool {
        self.more && self.paging == Paging::Idle && last + NEAR_END >= self.rows.len()
    }
}

/// The actor in the pane, from `get_actor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Details {
    /// Its type's name.
    pub type_name: String,
    /// The commits that wrote its state.
    pub version: i64,
    /// When it was created.
    pub created_at: Timestamp,
    /// The actor whose turn created it.
    pub parent: Option<ActorAddress>,
    /// The only claim owner that may run its turns.
    pub pinned_to: Option<NodeId>,
    /// True while its mailbox is paused.
    pub paused: bool,
    /// Its state, formatted off the main thread.
    pub state: Arc<Json>,
}

/// A message of the mailbox's first page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxRow {
    /// Its id.
    pub id: MessageId,
    /// Its message type.
    pub message_type: String,
    /// When it is due.
    pub deliver_at: Timestamp,
    /// Its claims so far, as this read saw them.
    pub attempt: u32,
    /// True for the message a turn is handling.
    pub running: bool,
}

/// The first [`MAILBOX_ROWS`] messages of the mailbox in delivery order, running and pending mixed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mailbox {
    /// The messages.
    pub rows: Vec<MailboxRow>,
    /// True when the mailbox has more.
    pub more: bool,
}

/// The first [`WATCH_ROWS`] watches of one kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Watches {
    /// The watches.
    pub items: Vec<WatchView>,
    /// True when there are more.
    pub more: bool,
}

/// One read of the pane: still out, or its result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Read<T> {
    /// Asked, not answered yet.
    #[default]
    Loading,
    /// The answer.
    Ready(T),
    /// The read failed: the text to show, without credentials.
    Failed(String),
}

impl<T> From<Part<T>> for Read<T> {
    fn from(part: Part<T>) -> Self {
        match part {
            Part::Ready(value) => Self::Ready(value),
            Part::Failed(error) => Self::Failed(error),
        }
    }
}

/// What the pane shows of the selected actor, each part with its own result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneView {
    /// Grows at every selection, Refresh and context switch.
    pub generation: u64,
    /// The selected actor; `None` leaves the pane empty.
    pub address: Option<ActorAddress>,
    /// `get_actor`; `None` when the actor does not exist.
    pub actor: Read<Option<Details>>,
    /// `turn`; `None` when it is idle.
    pub turn: Read<Option<RunningTurnView>>,
    /// The mailbox's first page.
    pub mailbox: Read<Mailbox>,
    /// The watches with the actor as the watcher.
    pub watching: Read<Watches>,
    /// The watches stored with the actor as their exact target; a watch on its type is not among them.
    pub watched_by: Read<Watches>,
}

impl PaneView {
    /// True when the actor is gone: the pane shows only that.
    pub fn not_found(&self) -> bool {
        matches!(self.actor, Read::Ready(None))
    }

    /// The running turn's message among the mailbox's first page, for its type and attempt; `None` when there is no
    /// turn, a read is out, or the message is not on that page.
    pub fn turn_message(&self) -> Option<&MailboxRow> {
        let (Read::Ready(Some(turn)), Read::Ready(mailbox)) = (&self.turn, &self.mailbox) else { return None };
        mailbox.rows.iter().find(|row| row.id == turn.message_id)
    }

    /// The pending messages among the mailbox's first page.
    pub fn pending(&self) -> impl Iterator<Item = &MailboxRow> {
        let rows = match &self.mailbox {
            Read::Ready(mailbox) => mailbox.rows.as_slice(),
            _ => &[],
        };
        rows.iter().filter(|row| !row.running)
    }
}

/// Where the next page starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Cursor {
    /// The first page is not read yet.
    #[default]
    Start,
    /// The server's cursor of the next page, passed back unchanged.
    Next(String),
    /// The last page is read.
    End,
}

#[derive(Default)]
struct ChildRead {
    cursor: Cursor,
    task: Option<JoinHandle<()>>,
}

/// The Actors page's reads, publishing a [`ListView`] and a [`PaneView`]. Owned by the session, which hands it the
/// client of the context in use.
pub struct Actors {
    shared: Arc<Shared>,
}

struct Shared {
    handle: Handle,
    limits: Limits,
    list: watch::Sender<ListView>,
    pane: watch::Sender<PaneView>,
    state: Mutex<State>,
}

struct State {
    client: Option<ActorClient>,
    /// True once the list was read for this client; [`Actors::show`] reads it only the first time.
    loaded: bool,
    list_generation: u64,
    prefix: String,
    cursor: Cursor,
    /// The page asked for: the generation and cursor it was asked under.
    page: Option<(u64, Option<String>)>,
    page_task: Option<JoinHandle<()>>,
    sample_task: Option<JoinHandle<()>>,
    debounce_task: Option<JoinHandle<()>>,
    /// A parent's cursor and request marker. The whole map is dropped with the list's generation.
    child_reads: BTreeMap<ActorPath, ChildRead>,
    /// The prefix last read, which owns the open paths kept across reloads.
    fold_prefix: String,
    pane_generation: u64,
    selected: Option<ActorPath>,
    pane_task: Option<JoinHandle<()>>,
    /// The tasks aborted so far, for tests that await their end.
    #[cfg(test)]
    aborted: Vec<JoinHandle<()>>,
}

impl State {
    fn abort(&mut self, task: Option<JoinHandle<()>>) {
        if let Some(task) = task {
            task.abort();
            #[cfg(test)]
            self.aborted.push(task);
        }
    }

    /// Drops every read of the list: a new generation, no page asked for, no pending search.
    fn supersede_list(&mut self) {
        self.list_generation += 1;
        self.page = None;
        for task in [self.page_task.take(), self.sample_task.take(), self.debounce_task.take()] {
            self.abort(task);
        }
        for (_, read) in std::mem::take(&mut self.child_reads) {
            self.abort(read.task);
        }
    }

    fn supersede_pane(&mut self) {
        self.pane_generation += 1;
        let task = self.pane_task.take();
        self.abort(task);
    }
}

impl Actors {
    /// No client yet; reads run on `handle` within `limits.deadline`, and a search waits `limits.debounce`.
    pub fn new(handle: Handle, limits: Limits) -> Self {
        let state = State {
            client: None,
            loaded: false,
            list_generation: 0,
            prefix: String::new(),
            cursor: Cursor::Start,
            page: None,
            page_task: None,
            sample_task: None,
            debounce_task: None,
            child_reads: BTreeMap::new(),
            fold_prefix: String::new(),
            pane_generation: 0,
            selected: None,
            pane_task: None,
            #[cfg(test)]
            aborted: Vec::new(),
        };
        let shared = Shared {
            handle,
            limits,
            list: watch::channel(ListView::default()).0,
            pane: watch::channel(PaneView::default()).0,
            state: Mutex::new(state),
        };
        Self { shared: Arc::new(shared) }
    }

    /// The list's views, newest last.
    pub fn list(&self) -> watch::Receiver<ListView> {
        self.shared.list.subscribe()
    }

    /// The pane's views, newest last.
    pub fn pane(&self) -> watch::Receiver<PaneView> {
        self.shared.pane.subscribe()
    }

    /// A context switch: drops every read, the rows, the prefix's rows and the selection. The list is read again at
    /// the next [`Self::show`].
    pub fn use_client(&self, client: Option<ActorClient>) {
        let mut state = self.shared.lock();
        state.supersede_list();
        state.supersede_pane();
        state.client = client;
        state.loaded = false;
        state.cursor = Cursor::Start;
        state.selected = None;
        let (generation, prefix) = (state.list_generation, state.prefix.clone());
        self.shared.list.send_replace(ListView { prefix, generation, ..ListView::default() });
        self.shared.pane.send_replace(PaneView { generation: state.pane_generation, ..PaneView::default() });
    }

    /// The page is shown: reads the list the first time for this client.
    pub fn show(&self) {
        let mut state = self.shared.lock();
        if !state.loaded && state.client.is_some() {
            self.shared.reload(&mut state, false);
        }
    }

    /// Refresh: reads the list again from the top under its prefix, with a new sample, and the pane again.
    pub fn refresh(&self) {
        let mut state = self.shared.lock();
        if state.client.is_none() {
            return;
        }
        self.shared.reload(&mut state, false);
        let selected = state.selected.clone();
        self.shared.select(&mut state, selected);
    }

    /// A key press in the search field: the rows go at once, and the new prefix is read once the field has been
    /// still for `limits.debounce`. A selection whose root is outside the prefix is cleared and its reads are dropped.
    pub fn search(&self, prefix: &str) {
        let mut state = self.shared.lock();
        state.prefix = prefix.to_owned();
        if state.selected.as_ref().is_some_and(|path| !path.0[0].as_str().starts_with(prefix)) {
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

    /// Opens a loaded row and asks for its children the first time it opens in `generation`.
    pub fn open(&self, generation: u64, path: &ActorPath) {
        let mut state = self.shared.lock();
        if state.list_generation != generation {
            return;
        }
        let mut first = false;
        self.shared.list.send_if_modified(|list| {
            if !list.contains(path) || !list.open.insert(path.clone()) {
                return false;
            }
            first = !list.children.contains_key(path);
            true
        });
        if first {
            self.shared.request_children(&mut state, path.clone());
        }
    }

    /// Closes a row without discarding its children or the fold state within them.
    pub fn close(&self, generation: u64, path: &ActorPath) {
        let state = self.shared.lock();
        if state.list_generation == generation {
            self.shared.list.send_if_modified(|list| list.open.remove(path));
        }
    }

    /// Asks for the next page of a parent's children. Returns true if the read started.
    pub fn more_children(&self, generation: u64, parent: &ActorPath) -> bool {
        self.page_children(generation, parent, |page| page.more && page.paging == Paging::Idle)
    }

    /// Asks again for a parent's failed page, from the cursor it kept. Returns true if the read started.
    pub fn retry_children(&self, generation: u64, parent: &ActorPath) -> bool {
        self.page_children(generation, parent, |page| matches!(page.paging, Paging::Failed(_)))
    }

    fn page_children(&self, generation: u64, parent: &ActorPath, can_page: impl FnOnce(&Children) -> bool) -> bool {
        let mut state = self.shared.lock();
        let list = self.shared.list.borrow();
        let ready = state.list_generation == generation
            && list.open.contains(parent)
            && list.children.get(parent).is_some_and(can_page);
        drop(list);
        if ready {
            self.shared.request_children(&mut state, parent.clone());
        }
        ready
    }

    /// Selects `path` and reads its actor into the pane; `None` empties the pane. Reads of an earlier selection are
    /// dropped, also when they are of the same actor.
    pub fn select(&self, path: Option<ActorPath>) {
        let mut state = self.shared.lock();
        self.shared.select(&mut state, path);
    }

    /// The window closes: every read is dropped.
    pub fn stop(&self) {
        let mut state = self.shared.lock();
        state.supersede_list();
        state.supersede_pane();
    }

    /// The selected row's path.
    pub fn selected(&self) -> Option<ActorPath> {
        self.shared.lock().selected.clone()
    }

    /// The tasks aborted so far; each ends once it has stopped.
    #[cfg(test)]
    pub fn take_aborted(&self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.shared.lock().aborted)
    }
}

impl Drop for Actors {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("inspector actors")
    }

    /// Replaces the rows under a new generation and reads the first page and the sample: now, or after the debounce.
    fn reload(self: &Arc<Self>, state: &mut State, debounce: bool) {
        state.supersede_list();
        state.loaded = true;
        state.cursor = Cursor::Start;
        let (generation, prefix) = (state.list_generation, state.prefix.clone());
        self.list.send_modify(|list| {
            *list = ListView {
                prefix,
                generation,
                paging: Paging::Loading,
                more: true,
                open: std::mem::take(&mut list.open),
                ..ListView::default()
            };
        });
        if debounce {
            let shared = self.clone();
            let wait = self.limits.debounce;
            state.debounce_task = Some(self.handle.spawn(async move {
                tokio::time::sleep(wait).await;
                let mut state = shared.lock();
                if state.list_generation == generation {
                    state.debounce_task = None;
                    shared.request_page(&mut state);
                    shared.request_sample(&mut state);
                }
            }));
        } else {
            self.request_page(state);
            self.request_sample(state);
        }
    }

    /// Asks for the page at the cursor under the current generation: at most one at a time, never one already out.
    fn request_page(self: &Arc<Self>, state: &mut State) {
        let Some(client) = state.client.clone() else { return };
        let cursor = match &state.cursor {
            Cursor::Start => None,
            Cursor::Next(cursor) => Some(cursor.clone()),
            Cursor::End => return,
        };
        if state.page.is_some() {
            return;
        }
        if state.fold_prefix != state.prefix {
            state.fold_prefix.clone_from(&state.prefix);
            self.list.send_modify(|list| list.open.clear());
        }
        let generation = state.list_generation;
        state.page = Some((generation, cursor.clone()));
        self.list.send_modify(|list| list.paging = Paging::Loading);
        let (shared, prefix) = (self.clone(), state.prefix.clone());
        state.page_task = Some(self.handle.spawn(async move {
            let deadline = Instant::now() + shared.limits.deadline;
            let read = client.list_actors_with_state(&prefix, cursor.as_deref(), Some(PAGE));
            let page = within(deadline, read).await;
            let mut state = shared.lock();
            if state.list_generation != generation || state.page != Some((generation, cursor)) {
                return;
            }
            state.page = None;
            state.page_task = None;
            match page {
                Part::Ready(page) => {
                    state.cursor = page.next.map_or(Cursor::End, Cursor::Next);
                    let more = state.cursor != Cursor::End;
                    let mut reopen = Vec::new();
                    shared.list.send_modify(|list| {
                        let first_new = list.rows.len();
                        list.rows.extend(page.items.into_iter().map(ActorRow::from));
                        list.paging = Paging::Idle;
                        list.more = more;
                        retain_open(&mut list.open, &[], list.rows.last().map(|row| &row.address), more, |address| {
                            list.rows.binary_search_by(|row| row.address.cmp(address)).is_ok()
                        });
                        reopen.extend(list.rows[first_new..].iter().filter_map(|row| {
                            let path = ActorPath::root(row.address.clone());
                            list.open.contains(&path).then_some(path)
                        }));
                    });
                    for parent in reopen {
                        shared.request_children(&mut state, parent);
                    }
                }
                Part::Failed(error) => shared.list.send_modify(|list| list.paging = Paging::Failed(error)),
            }
        }));
    }

    fn request_children(self: &Arc<Self>, state: &mut State, parent: ActorPath) {
        let Some(client) = state.client.clone() else { return };
        let read = state.child_reads.entry(parent.clone()).or_default();
        if read.task.is_some() || read.cursor == Cursor::End {
            return;
        }
        self.list.send_modify(|list| {
            list.children.entry(parent.clone()).or_default().paging = Paging::Loading;
        });
        let generation = state.list_generation;
        let (shared, path, asked) = (self.clone(), parent, read.cursor.clone());
        read.task = Some(self.handle.spawn(async move {
            let cursor = match &asked {
                Cursor::Start => None,
                Cursor::Next(cursor) => Some(cursor.as_str()),
                Cursor::End => unreachable!("the last page was read"),
            };
            let deadline = Instant::now() + shared.limits.deadline;
            // Error bodies and request URLs can carry credentials. Report only the status of a failed read.
            let page = match timeout_at(deadline, client.children(path.address(), cursor, Some(PAGE))).await {
                Ok(Ok(page)) => Part::Ready(page),
                Ok(Err(error)) => Part::Failed(match error.status() {
                    Some(status) => format!("HTTP {status}"),
                    None => "The server's response could not be read.".into(),
                }),
                Err(_) => Part::Failed(TIMED_OUT.into()),
            };
            let mut state = shared.lock();
            if state.list_generation != generation {
                return;
            }
            let Some(read) = state.child_reads.get_mut(&path) else { return };
            if read.cursor != asked || read.task.is_none() {
                return;
            }
            read.task = None;
            let missing = matches!(page, Part::Ready(None));
            let descendant = |other: &ActorPath| other != &path && other.0.starts_with(&path.0);
            let mut reopen = Vec::new();
            shared.list.send_modify(|list| {
                let children = list.children.get_mut(&path).expect("the parent has a page in flight");
                let first_new = children.rows.len();
                match page {
                    Part::Ready(Some(page)) => {
                        children.rows.extend(page.items);
                        read.cursor = page.next.map_or(Cursor::End, Cursor::Next);
                    }
                    Part::Ready(None) => {
                        children.rows.clear();
                        children.not_found = true;
                        read.cursor = Cursor::End;
                    }
                    Part::Failed(error) => {
                        children.paging = Paging::Failed(error);
                        return;
                    }
                }
                children.paging = Paging::Idle;
                children.more = read.cursor != Cursor::End;
                retain_open(&mut list.open, &path.0, children.rows.last(), children.more, |address| {
                    children.rows.binary_search(address).is_ok()
                });
                reopen.extend(children.rows.iter().skip(first_new).filter_map(|address| {
                    let child = path.child(address.clone());
                    list.open.contains(&child).then_some(child)
                }));
                if missing {
                    list.children.retain(|other, _| !descendant(other));
                }
            });
            if missing {
                let reads: Vec<_> = state.child_reads.extract_if(.., |other, _| descendant(other)).collect();
                for (_, read) in reads {
                    state.abort(read.task);
                }
            }
            for child in reopen {
                shared.request_children(&mut state, child);
            }
        }));
    }

    /// Reads the first [`SAMPLE`] running turns under the prefix, once per reload.
    fn request_sample(self: &Arc<Self>, state: &mut State) {
        let Some(client) = state.client.clone() else { return };
        let (shared, prefix, generation) = (self.clone(), state.prefix.clone(), state.list_generation);
        state.sample_task = Some(self.handle.spawn(async move {
            let deadline = Instant::now() + shared.limits.deadline;
            let sample = within(deadline, client.turns(None, &prefix, None, Some(SAMPLE))).await;
            let running = match sample {
                Part::Ready(page) => Running::Sample {
                    partial: page.next.is_some(),
                    actors: page.items.into_iter().map(|turn| turn.actor).collect(),
                },
                Part::Failed(error) => Running::Unknown(error),
            };
            let mut state = shared.lock();
            if state.list_generation == generation {
                state.sample_task = None;
                shared.list.send_modify(|list| list.running = running);
            }
        }));
    }

    /// Selects a row under a new pane generation and starts its actor's reads, each published as it answers.
    fn select(self: &Arc<Self>, state: &mut State, path: Option<ActorPath>) {
        state.supersede_pane();
        let address = path.as_ref().map(|path| path.address().clone());
        state.selected = path;
        let generation = state.pane_generation;
        let reading = address.is_some() && state.client.is_some();
        self.pane.send_replace(match &address {
            Some(_) if !reading => PaneView {
                generation,
                address: address.clone(),
                actor: Read::Failed("no server".into()),
                turn: Read::Failed("no server".into()),
                mailbox: Read::Failed("no server".into()),
                watching: Read::Failed("no server".into()),
                watched_by: Read::Failed("no server".into()),
            },
            _ => PaneView { generation, address: address.clone(), ..PaneView::default() },
        });
        let (Some(address), Some(client)) = (address, state.client.clone()) else { return };
        let shared = self.clone();
        state.pane_task = Some(self.handle.spawn(async move { shared.read_pane(client, address, generation).await }));
    }

    async fn read_pane(&self, client: ActorClient, address: ActorAddress, generation: u64) {
        let deadline = Instant::now() + self.limits.deadline;
        let apply = |change: &dyn Fn(&mut PaneView)| {
            let state = self.lock();
            if state.pane_generation == generation {
                self.pane.send_modify(change);
            }
        };
        let actor = async {
            let read: Read<_> = within(deadline, client.get_actor(&address)).await.into();
            let read = match read {
                Read::Ready(view) => Read::Ready(view.map(details)),
                Read::Failed(error) => Read::Failed(error),
                Read::Loading => Read::Loading,
            };
            apply(&|pane| pane.actor = read.clone());
        };
        let turn = async {
            let read: Read<_> = within(deadline, client.turn(&address)).await.into();
            apply(&|pane| pane.turn = read.clone());
        };
        let mailbox = async {
            let read = within(deadline, client.mailbox(&address, None, Some(MAILBOX_ROWS))).await;
            let read: Read<Mailbox> = match read {
                Part::Ready(Some(page)) => Read::Ready(Mailbox {
                    more: page.next.is_some(),
                    rows: page
                        .items
                        .into_iter()
                        .map(|row| MailboxRow {
                            running: row.state == MessageState::Running,
                            id: row.message.id,
                            message_type: row.message.message_type,
                            deliver_at: row.message.deliver_at,
                            attempt: row.message.attempt,
                        })
                        .collect(),
                }),
                // The actor is gone; `get_actor` says so for the whole pane.
                Part::Ready(None) => Read::Ready(Mailbox::default()),
                Part::Failed(error) => Read::Failed(error),
            };
            apply(&|pane| pane.mailbox = read.clone());
        };
        let watching = async {
            let read = within(deadline, client.watches(Some(&address), None, None, Some(WATCH_ROWS))).await;
            let read: Read<_> = read.map(|page| Watches { more: page.next.is_some(), items: page.items }).into();
            apply(&|pane| pane.watching = read.clone());
        };
        let watched_by = async {
            let target = WatchTarget::Actor(address.clone());
            let read = within(deadline, client.watches(None, Some(&target), None, Some(WATCH_ROWS))).await;
            let read: Read<_> = read.map(|page| Watches { more: page.next.is_some(), items: page.items }).into();
            apply(&|pane| pane.watched_by = read.clone());
        };
        tokio::join!(actor, turn, mailbox, watching, watched_by);
    }
}

/// Once a page passes a missing row (or ends its list), forget the row and any saved open paths through it.
fn retain_open(
    open: &mut BTreeSet<ActorPath>,
    parent: &[ActorAddress],
    last: Option<&ActorAddress>,
    more: bool,
    found: impl Fn(&ActorAddress) -> bool,
) {
    open.retain(|path| {
        let address = path.0.strip_prefix(parent).and_then(|tail| tail.first());
        address.is_none_or(|address| (more && last.is_none_or(|last| address > last)) || found(address))
    });
}

/// The pane's details of `view`, with its state formatted; the rest of the view is dropped.
fn details(view: ActorView) -> Details {
    Details {
        type_name: view.type_name,
        version: view.version,
        created_at: view.created_at,
        parent: view.parent,
        pinned_to: view.pinned_to,
        paused: view.paused,
        state: Arc::new(json::format(&view.state)),
    }
}
