//! The Actors page: the paged list of actors under a prefix with its sample of running turns, and the pane's reads of
//! the selected actor. Every read carries the generation it was asked under and is dropped when that no longer
//! matches: the list's generation grows at every key press in the search field, on Refresh and on a context switch;
//! the pane's at every selection, on Refresh and on a context switch.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard};

use cicada::ActorAddress;
use cicada::api::{ActorView, MessageState, PageView, RunningTurnView, WatchView};
use cicada::store::{MessageId, NodeId};
use cicada::time::Timestamp;
use cicada::watch::WatchTarget;
use cicada_client::ActorClient;
use futures::future::join_all;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout_at};

use crate::json::{self, Json};
use crate::model::{Limits, Part, TIMED_OUT, within};
pub use crate::pager::Paging;
use crate::pager::{Cursor, Landed, NEAR_END, PAGE, Paged, Pager, read_range};
use crate::refresh::{Budget, Readiness, Refresh, RefreshPage, Stale, as_of};

/// The running turns read once per reload of the list to mark rows running.
pub const SAMPLE: usize = 200;
/// The mailbox rows the pane reads.
pub const MAILBOX_ROWS: usize = 50;
/// The watches of each kind the pane reads.
pub const WATCH_ROWS: usize = 50;
const REFRESH_PARENTS: usize = 20;

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
    stale: Option<ChildStale>,
    /// Why the first page could not be refreshed, alongside the cached children.
    pub refresh_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildStale {
    FirstPage,
    Tail,
}

impl Children {
    /// True while any loaded children are from before the last soft refresh.
    pub fn not_refreshed(&self) -> bool {
        self.stale.is_some()
    }
}

/// What the rendered outline reported when a soft refresh was requested.
#[derive(Default)]
pub struct OutlineViewport {
    /// The generation the view rendered.
    pub generation: u64,
    /// The first visible row and the visible range, including status rows.
    pub visible: Range<usize>,
    /// Expanded actor paths and their row indices in screen order, excluding folds under collapsed rows.
    pub open: Vec<(usize, ActorPath)>,
}

impl OutlineViewport {
    fn parents(mut self, list: &ListView) -> Vec<ActorPath> {
        if self.generation != list.generation {
            return Vec::new();
        }
        self.open.sort_by_key(|(row, _)| {
            if *row < self.visible.start {
                (self.visible.start - row, true)
            } else {
                (row.saturating_add(1).saturating_sub(self.visible.end), false)
            }
        });
        let mut parents: Vec<_> = self
            .open
            .into_iter()
            .map(|(_, path)| path)
            .filter(|path| list.can_refresh_children(path))
            .take(REFRESH_PARENTS)
            .collect();
        // Select the nearest parents, then read ancestors before descendants.
        parents.sort_by_key(|path| path.0.len());
        parents
    }
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
    stale: Option<Stale>,
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

    fn can_refresh_children(&self, path: &ActorPath) -> bool {
        if self.children.get(path).is_some_and(|page| matches!(page.paging, Paging::Failed(_))) {
            return false;
        }
        let mut path = path.clone();
        loop {
            if !self.open.contains(&path) || !self.contains(&path) {
                return false;
            }
            if path.is_root() {
                return true;
            }
            path.0.pop();
        }
    }

    /// The footer under the table: what the running status covers, when it is not every running turn.
    pub fn footer(&self) -> Option<String> {
        let running = match &self.running {
            Running::Sample { partial: true, .. } => {
                Some(format!("The running status covers the first {SAMPLE} running turns."))
            }
            Running::Unknown(error) => Some(format!("The running status is unknown: {error}")),
            Running::Loading | Running::Sample { partial: false, .. } => None,
        };
        let stale = self.stale.as_ref().map(|mark| match &mark.error {
            Some(error) => format!("{} ({error})", mark.text()),
            None => mark.text(),
        });
        match (running, stale) {
            (Some(running), Some(stale)) => Some(format!("{running} · {stale}")),
            (running, stale) => running.or(stale),
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

/// A Show Actor waiting for the first page of the generation it reloaded the list in.
struct Jump {
    generation: u64,
    actor: ActorAddress,
}

#[derive(Default)]
struct ChildRead {
    cursor: Cursor,
    covered: Option<ActorAddress>,
    task: Option<(u64, JoinHandle<()>)>,
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
    /// The root list's paging; its generation is the list's.
    pager: Pager,
    root_covered: Option<ActorAddress>,
    child_ticket: u64,
    viewport: OutlineViewport,
    deferred_children: BTreeMap<ActorPath, Cursor>,
    prefix: String,
    sample_task: Option<JoinHandle<()>>,
    /// A parent's cursor and request marker. The whole map is dropped with the list's generation.
    child_reads: BTreeMap<ActorPath, ChildRead>,
    /// The prefix last read, which owns the open paths kept across reloads.
    fold_prefix: String,
    pane_generation: u64,
    selected: Option<ActorPath>,
    pane_task: Option<JoinHandle<()>>,
    jump: Option<Jump>,
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

    fn list_generation(&self) -> u64 {
        self.pager.generation()
    }

    /// Drops every read of the list: a new generation from the first page, no page asked for, no pending search.
    fn supersede_list(&mut self) {
        let [page, debounce] = self.pager.restart();
        self.root_covered = None;
        self.viewport = OutlineViewport::default();
        self.deferred_children.clear();
        for task in [page, self.sample_task.take(), debounce] {
            self.abort(task);
        }
        for (_, read) in std::mem::take(&mut self.child_reads) {
            self.abort(read.task.map(|(_, task)| task));
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
            pager: Pager::default(),
            root_covered: None,
            child_ticket: 0,
            viewport: OutlineViewport::default(),
            deferred_children: BTreeMap::new(),
            prefix: String::new(),
            sample_task: None,
            child_reads: BTreeMap::new(),
            fold_prefix: String::new(),
            pane_generation: 0,
            selected: None,
            pane_task: None,
            jump: None,
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
        state.selected = None;
        state.jump = None;
        let (generation, prefix) = (state.list_generation(), state.prefix.clone());
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
        state.jump = None;
        self.shared.reload(&mut state, false);
        let selected = state.selected.clone();
        self.shared.select(&mut state, selected);
    }

    /// Re-reads the loaded range of the Actors page without replacing its generation.
    pub fn soft_refresh(&self, viewport: OutlineViewport) {
        let mut state = self.shared.lock();
        state.viewport = viewport;
        self.shared.request_locked(&mut state);
    }

    /// Cancels the soft refresh when leaving the page.
    pub fn cancel_soft_refresh(&self) {
        let mut state = self.shared.lock();
        state.pager.cancel_refresh();
        state.viewport = OutlineViewport::default();
        self.shared.after(&mut state);
    }

    /// A key press in the search field: the rows go at once, and the new prefix is read once the field has been
    /// still for `limits.debounce`. A selection whose root is outside the prefix is cleared and its reads are dropped.
    pub fn search(&self, prefix: &str) {
        let mut state = self.shared.lock();
        state.prefix = prefix.to_owned();
        state.jump = None;
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
        if state.list_generation() != generation {
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
        if state.list_generation() == generation {
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
        let ready = state.list_generation() == generation
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
        state.jump = None;
        self.shared.select(&mut state, path);
    }

    /// Show Actor: unselects, searches for the exact address of `actor` at once, and once the first page of that
    /// search has loaded selects the root row with exactly that address; when the page has no such row, the pane says
    /// the actor is not found. A failed page selects nothing and keeps the target for Retry. A new search, Refresh,
    /// a context switch, [`Self::cancel_jump`] or a selection cancels it.
    pub fn jump(&self, actor: ActorAddress) {
        let mut state = self.shared.lock();
        state.prefix = actor.to_string();
        state.jump = None;
        self.shared.select(&mut state, None);
        if state.client.is_none() {
            self.shared.list.send_modify(|list| list.prefix = actor.to_string());
            return;
        }
        self.shared.reload(&mut state, false);
        state.jump = Some(Jump { generation: state.list_generation(), actor });
    }

    /// Cancels a Show Actor still waiting for its page; for leaving the page.
    pub fn cancel_jump(&self) {
        self.shared.lock().jump = None;
    }

    /// True while a Show Actor waits for its page.
    #[cfg(test)]
    pub fn jumping(&self) -> bool {
        self.shared.lock().jump.is_some()
    }

    /// The window closes: every read is dropped.
    pub fn stop(&self) {
        let mut state = self.shared.lock();
        state.jump = None;
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

impl Paged for Shared {
    type State = State;

    fn handle(&self) -> &Handle {
        &self.handle
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("inspector actors")
    }

    fn pager(state: &mut State) -> &mut Pager {
        &mut state.pager
    }
}

impl RefreshPage for Shared {
    type State = State;

    fn lock_refresh(&self) -> MutexGuard<'_, State> {
        self.lock()
    }

    fn refresh(state: &mut State) -> &mut Refresh {
        &mut state.pager.refresh
    }

    fn runtime(&self) -> (&Handle, std::time::Duration) {
        (&self.handle, self.limits.deadline)
    }

    fn readiness(&self, state: &State) -> Readiness {
        match self.list.borrow().paging {
            Paging::Loading => Readiness::Wait,
            Paging::Idle if state.child_reads.values().any(|read| read.task.is_some()) => Readiness::Wait,
            Paging::Idle if state.loaded && state.client.is_some() => Readiness::Ready,
            _ => Readiness::Skip,
        }
    }

    async fn run(self: &Arc<Self>, ticket: u64, budget: Budget) {
        let (client, prefix, last, parents) = {
            let mut state = self.lock();
            if !state.pager.refresh.current(ticket) {
                return;
            }
            let Some(client) = state.client.clone() else { return };
            let list = self.list.borrow();
            let last = list.rows.last().map(|row| row.address.clone());
            let parents = std::mem::take(&mut state.viewport).parents(&list);
            drop(list);
            self.list.send_if_modified(|list| {
                for page in list.children.values_mut() {
                    page.stale = Some(ChildStale::FirstPage);
                }
                !list.children.is_empty()
            });
            (client, state.prefix.clone(), last, parents)
        };
        let range = read_range(
            &budget,
            |cursor| {
                let (client, prefix) = (&client, &prefix);
                async move { client.list_actors_with_state(prefix, cursor.as_deref(), Some(PAGE)).await }
            },
            last.as_ref(),
            |row| row.address.clone(),
        )
        .await;
        {
            let mut state = self.lock();
            if !state.pager.refresh.current(ticket) {
                return;
            }
            let old_time = as_of(self.list.borrow().stale.as_ref(), state.pager.read_at());
            let mut kept = false;
            if let Some(page) = range.page {
                let more = page.next.is_some();
                state.pager.accept_cursor(page.next);
                state.root_covered = None;
                kept = self.merge_roots(&mut state, page.items, more);
            }
            let stale = (kept || range.error.is_some()).then_some(Stale {
                as_of: old_time,
                capped: kept && range.error.is_none(),
                error: range.error,
            });
            self.list.send_modify(|list| list.stale = stale);
        }
        self.refresh_children(ticket, &budget, &client, &parents).await;
    }

    fn after(self: &Arc<Self>, state: &mut State) {
        if state.pager.take_deferred_page() {
            self.request_page(state);
        }
        for (path, asked) in std::mem::take(&mut state.deferred_children) {
            let open = {
                let list = self.list.borrow();
                list.open.contains(&path) && list.contains(&path)
            };
            if open
                && (asked != Cursor::Start
                    || state.child_reads.get(&path).is_none_or(|read| read.cursor == Cursor::Start))
            {
                self.request_children(state, path);
            }
        }
    }
}

impl Shared {
    async fn refresh_children(
        self: &Arc<Self>,
        ticket: u64,
        budget: &Budget,
        client: &ActorClient,
        parents: &[ActorPath],
    ) {
        for depth in parents.chunk_by(|a, b| a.0.len() == b.0.len()) {
            // Each depth completes before any descendant can start, so the check uses its parent's new rows.
            join_all(depth.iter().map(|path| async move {
                {
                    let state = self.lock();
                    if !state.pager.refresh.current(ticket) || !self.list.borrow().can_refresh_children(path) {
                        return;
                    }
                }
                let page = budget.read(client.children(path.address(), None, Some(PAGE))).await;
                let mut state = self.lock();
                if state.pager.refresh.current(ticket) {
                    self.land_children(&mut state, path, page, true, true);
                }
            }))
            .await;
        }
    }

    /// Replaces the rows under a new generation and reads the first page and the sample: now, or after the debounce.
    fn reload(self: &Arc<Self>, state: &mut State, debounce: bool) {
        state.supersede_list();
        state.loaded = true;
        let (generation, prefix) = (state.list_generation(), state.prefix.clone());
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
            Pager::debounce(self, state, self.limits, |shared, state| {
                shared.request_page(state);
                shared.request_sample(state);
            });
        } else {
            self.request_page(state);
            self.request_sample(state);
        }
    }

    /// Asks for the page at the cursor under the current generation: at most one at a time, never one already out.
    fn request_page(self: &Arc<Self>, state: &mut State) {
        let Some(client) = state.client.clone() else { return };
        if !state.pager.can_ask() {
            return;
        }
        if state.fold_prefix != state.prefix {
            state.fold_prefix.clone_from(&state.prefix);
            self.list.send_modify(|list| list.open.clear());
        }
        let prefix = state.prefix.clone();
        let read = move |cursor: Option<String>| async move {
            client.list_actors_with_state(&prefix, cursor.as_deref(), Some(PAGE)).await
        };
        if Pager::request(self, state, self.limits, read, Self::land_page) {
            self.list.send_modify(|list| list.paging = Paging::Loading);
        }
    }

    /// Appends a root page that answered, reopening its rows that were open; or shows why it failed.
    fn land_page(self: &Arc<Self>, state: &mut State, landed: Landed<ActorView>) {
        let (items, more) = match landed.page {
            Part::Ready(page) => page,
            Part::Failed(error) => {
                self.list.send_modify(|list| list.paging = Paging::Failed(error));
                self.resume(state);
                return;
            }
        };
        self.merge_roots(state, items, more);
        if landed.first && state.jump.as_ref().is_some_and(|jump| jump.generation == landed.generation) {
            let actor = state.jump.take().expect("a jump waits").actor;
            if self.list.borrow().root_index(&actor).is_some() {
                self.select(state, Some(ActorPath::root(actor)));
            } else {
                self.not_found(state, actor);
            }
        }
        self.resume(state);
    }

    fn merge_roots(self: &Arc<Self>, state: &mut State, fresh: Vec<ActorView>, more: bool) -> bool {
        let mut reopen = Vec::new();
        let mut clear = false;
        let mut kept = false;
        self.list.send_modify(|list| {
            kept = merge_page(
                &mut list.rows,
                fresh.into_iter().map(ActorRow::from).collect(),
                &mut state.root_covered,
                more,
                |row| &row.address,
            );
            list.paging = Paging::Idle;
            list.more = more;
            let last = state.root_covered.clone();
            clear = prune_paths(list, state, &[], last.as_ref(), more, &mut reopen);
        });
        self.after_prune(state, clear, reopen);
        kept
    }

    /// The pane of a Show Actor whose actor is not in the list: "not found", with no row selected and no read.
    fn not_found(&self, state: &mut State, actor: ActorAddress) {
        state.supersede_pane();
        state.selected = None;
        self.pane.send_replace(PaneView {
            generation: state.pane_generation,
            address: Some(actor),
            actor: Read::Ready(None),
            turn: Read::Ready(None),
            mailbox: Read::Ready(Mailbox::default()),
            watching: Read::Ready(Watches::default()),
            watched_by: Read::Ready(Watches::default()),
        });
    }

    fn request_children(self: &Arc<Self>, state: &mut State, parent: ActorPath) {
        let Some(client) = state.client.clone() else { return };
        if state.pager.refresh.running() {
            let cursor = state.child_reads.get(&parent).map_or(Cursor::Start, |read| read.cursor.clone());
            state.deferred_children.insert(parent, cursor);
            return;
        }
        let generation = state.list_generation();
        let read = state.child_reads.entry(parent.clone()).or_default();
        if read.task.is_some() || read.cursor == Cursor::End {
            return;
        }
        self.list.send_modify(|list| {
            list.children.entry(parent.clone()).or_default().paging = Paging::Loading;
        });
        state.child_ticket += 1;
        let ticket = state.child_ticket;
        let (shared, path, asked) = (self.clone(), parent, read.cursor.clone());
        let task = self.handle.spawn(async move {
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
            if state.list_generation() != generation {
                return;
            }
            let Some(read) = state.child_reads.get_mut(&path) else { return };
            if read.task.as_ref().is_none_or(|(asked, _)| *asked != ticket) {
                return;
            }
            read.task = None;
            shared.land_children(&mut state, &path, page, asked == Cursor::Start, false);
            shared.resume(&mut state);
        });
        read.task = Some((ticket, task));
    }

    fn land_children(
        self: &Arc<Self>,
        state: &mut State,
        parent: &ActorPath,
        page: Part<Option<PageView<ActorAddress>>>,
        first: bool,
        refreshing: bool,
    ) {
        let mut reopen = Vec::new();
        let mut clear = false;
        self.list.send_modify(|list| {
            let children = list.children.entry(parent.clone()).or_default();
            let read = state.child_reads.entry(parent.clone()).or_default();
            match page {
                Part::Ready(Some(page)) => {
                    if first {
                        read.covered = None;
                    }
                    children.more = page.next.is_some();
                    let kept = merge_page(&mut children.rows, page.items, &mut read.covered, children.more, |row| row);
                    if first || children.stale == Some(ChildStale::Tail) {
                        children.stale = kept.then_some(ChildStale::Tail);
                        children.refresh_error = None;
                    }
                    read.cursor = page.next.map_or(Cursor::End, Cursor::Next);
                    children.not_found = false;
                }
                Part::Ready(None) => {
                    children.rows.clear();
                    children.more = false;
                    children.not_found = true;
                    children.stale = None;
                    children.refresh_error = None;
                    read.cursor = Cursor::End;
                }
                Part::Failed(error) if refreshing => {
                    children.stale = Some(ChildStale::FirstPage);
                    children.refresh_error = Some(error);
                    return;
                }
                Part::Failed(error) => {
                    children.paging = Paging::Failed(error);
                    return;
                }
            }
            children.paging = Paging::Idle;
            let (last, more) = (read.covered.clone(), children.more);
            clear = prune_paths(list, state, &parent.0, last.as_ref(), more, &mut reopen);
        });
        self.after_prune(state, clear, reopen);
    }

    fn after_prune(self: &Arc<Self>, state: &mut State, clear: bool, reopen: Vec<ActorPath>) {
        if clear {
            self.select(state, None);
        }
        for path in reopen {
            self.request_children(state, path);
        }
    }

    /// Reads the first [`SAMPLE`] running turns under the prefix, once per reload.
    fn request_sample(self: &Arc<Self>, state: &mut State) {
        let Some(client) = state.client.clone() else { return };
        let (shared, prefix, generation) = (self.clone(), state.prefix.clone(), state.list_generation());
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
            if state.list_generation() == generation {
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

/// Replaces the newly covered address interval, preserving the prefix before the cursor and any unseen tail.
fn merge_page<T>(
    rows: &mut Vec<T>,
    mut fresh: Vec<T>,
    covered: &mut Option<ActorAddress>,
    more: bool,
    address: impl Fn(&T) -> &ActorAddress,
) -> bool {
    let start = covered.as_ref().map_or(0, |after| rows.partition_point(|row| address(row) <= after));
    fresh.dedup_by(|a, b| address(a) == address(b));
    if let Some(last) = fresh.last() {
        *covered = Some(address(last).clone());
    }
    let end = if more {
        covered.as_ref().map_or(0, |last| rows.partition_point(|row| address(row) <= last))
    } else {
        rows.len()
    };
    let kept = end < rows.len();
    rows.splice(start..end, fresh);
    kept
}

/// Once a page passes a missing row, forget cached descendants, folds and outstanding reads through that row.
fn prune_paths(
    list: &mut ListView,
    state: &mut State,
    parent: &[ActorAddress],
    last: Option<&ActorAddress>,
    more: bool,
    reopen: &mut Vec<ActorPath>,
) -> bool {
    let current = (!parent.is_empty()).then(|| ActorPath(parent.to_vec()));
    let children = current.as_ref().and_then(|path| list.children.get(path));
    let gone = |path: &ActorPath| {
        path.0.strip_prefix(parent).and_then(|tail| tail.first()).is_some_and(|address| {
            (!more || last.is_some_and(|last| address <= last))
                && match children {
                    Some(page) => page.rows.binary_search(address).is_err(),
                    None => list.root_index(address).is_none(),
                }
        })
    };
    let clear = state.selected.as_ref().is_some_and(&gone);
    // Collect before mutating the caches used to determine which paths are still present.
    let removed: BTreeSet<_> = list
        .open
        .iter()
        .chain(list.children.keys())
        .chain(state.child_reads.keys())
        .chain(state.deferred_children.keys())
        .filter(|path| gone(path))
        .cloned()
        .collect();
    for path in removed {
        list.open.remove(&path);
        list.children.remove(&path);
        state.deferred_children.remove(&path);
        if let Some(read) = state.child_reads.remove(&path) {
            state.abort(read.task.map(|(_, task)| task));
        }
    }
    reopen.extend(
        list.open
            .iter()
            .filter(|path| {
                path.0.len() == parent.len() + 1
                    && path.0.starts_with(parent)
                    && !list.children.contains_key(*path)
                    && list.contains(path)
            })
            .cloned(),
    );
    clear
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

#[cfg(test)]
mod refresh_tests {
    use super::*;

    fn address(key: &str) -> ActorAddress {
        ActorAddress::parse(&format!("counter/{key}")).unwrap()
    }

    fn page(keys: &[&str], next: Option<&str>) -> Part<Option<PageView<ActorAddress>>> {
        Part::Ready(Some(PageView {
            items: keys.iter().map(|key| address(key)).collect(),
            next: next.map(str::to_owned),
        }))
    }

    #[test]
    fn the_nearest_twenty_parents_include_visible_rows_and_break_ties_below_first() {
        let paths: Vec<_> = (0..30).map(|row| ActorPath::root(address(&format!("{row:02}")))).collect();
        let mut list = ListView {
            rows: paths
                .iter()
                .map(|path| ActorRow {
                    address: path.address().clone(),
                    type_name: "counter".into(),
                    version: 0,
                    created_at: chrono::Utc::now(),
                    paused: false,
                })
                .collect(),
            open: paths.iter().cloned().collect(),
            ..ListView::default()
        };
        list.children
            .insert(paths[15].clone(), Children { paging: Paging::Failed("HTTP 500".into()), ..Children::default() });
        let viewport = OutlineViewport {
            generation: list.generation,
            visible: 14..17,
            open: paths.iter().cloned().enumerate().collect(),
        };
        let expected = [14, 16, 17, 13, 18, 12, 19, 11, 20, 10, 21, 9, 22, 8, 23, 7, 24, 6, 25, 5];
        assert_eq!(viewport.parents(&list), expected.map(|row| paths[row].clone()));
    }

    #[tokio::test(start_paused = true)]
    async fn children_merge_the_new_range_and_prune_an_exhausted_tail() {
        let actors = Actors::new(Handle::current(), Limits::DEFAULT);
        let shared = &actors.shared;
        let mut state = shared.lock();
        let parent = ActorPath::root(address("parent"));
        shared.land_children(&mut state, &parent, page(&["a", "b", "c", "d", "e"], None), true, false);
        let removed = parent.child(address("b"));
        let retained = parent.child(address("c"));
        let grandchild = removed.child(address("grandchild"));
        shared.land_children(&mut state, &removed, page(&["grandchild"], None), true, false);
        shared.land_children(&mut state, &grandchild, Part::Failed("HTTP 500".into()), true, false);
        shared.list.send_modify(|list| list.open.extend([parent.clone(), removed.clone(), grandchild.clone()]));
        shared.select(&mut state, Some(grandchild.clone()));
        let read = Handle::current().spawn(std::future::pending());
        state.child_reads.get_mut(&grandchild).unwrap().task = Some((99, read));

        shared.land_children(&mut state, &parent, page(&["a", "bb"], Some("new")), true, true);
        let list = shared.list.borrow();
        assert_eq!(list.children[&parent].rows, ["a", "bb", "c", "d", "e"].map(address));
        assert_eq!(list.open, BTreeSet::from([parent.clone()]));
        assert!(!list.children.contains_key(&removed) && !list.children.contains_key(&grandchild));
        assert!(!state.child_reads.contains_key(&removed) && !state.child_reads.contains_key(&grandchild));
        assert!(state.selected.is_none() && shared.pane.borrow().address.is_none());
        assert_eq!(state.child_reads[&parent].cursor, Cursor::Next("new".into()));
        drop(list);

        shared.land_children(&mut state, &parent, page(&["c", "f"], None), false, false);
        assert_eq!(
            shared.list.borrow().children[&parent].rows,
            [address("a"), address("bb"), retained.address().clone(), address("f")]
        );
        assert_eq!(state.child_reads[&parent].cursor, Cursor::End);
        let [aborted] = std::mem::take(&mut state.aborted).try_into().ok().unwrap();
        drop(state);
        assert!(aborted.await.unwrap_err().is_cancelled());
    }
}
