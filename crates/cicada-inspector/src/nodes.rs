//! The Nodes page: every node, read with `/nodes` into a cache of the page's own and filtered there by id prefix; and
//! the pane of the selected node, read with `/nodes/{node}`, with its running turns. The cache is read on the first
//! visit, on the page's Refresh and after a context switch (at once when the page is visible, else at its next visit),
//! never on the Overview's refresh. Health and the last heartbeat show as the server gave them: the answer carries no
//! store time, so no age is measured on the Mac's clock. The list's generation grows at every read and context switch;
//! the pane's at every selection, Refresh, context switch, and when the filter hides the selected node.

use std::sync::{Arc, Mutex, MutexGuard};

use cicada::api::{NodeDetailView, NodeView};
use cicada::runtime::NodeHealth;
use cicada::store::NodeId;
use cicada::time::Timestamp;
use cicada_client::ActorClient;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::model::{Limits, Part, within};

/// The footer's note on how current the health is.
pub const AS_OF: &str = "as of the last read";

/// The state of the cached list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NodesRead {
    /// Not read for this context yet.
    #[default]
    NotRead,
    /// A read is out.
    Loading,
    /// The last read answered, at this time of the Mac's clock: for the footer's tooltip only.
    Ready(Timestamp),
    /// The last read failed: the text to show; no stale list is kept, and Retry reads again.
    Failed(String),
}

/// What the Nodes table shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodesView {
    /// The id prefix the rows are filtered by.
    pub prefix: String,
    /// Grows at every read and context switch.
    pub generation: u64,
    /// The cached nodes whose id starts with the prefix, in the server's order; shared with the cache.
    pub rows: Vec<Arc<NodeView>>,
    /// How many nodes the cache holds, also those the filter hides.
    pub loaded: usize,
    /// The state of the last read.
    pub read: NodesRead,
}

impl NodesView {
    /// The row of the node `id`.
    pub fn index(&self, id: &NodeId) -> Option<usize> {
        self.rows.iter().position(|row| &row.id == id)
    }
}

/// A read of the selected node.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum NodeRead {
    /// The read is out.
    #[default]
    Loading,
    /// The node with its running turns.
    Found(Arc<NodeDetailView>),
    /// No such node: its row was removed.
    NotFound,
    /// The read failed: the text to show; Retry reads the selected node again.
    Failed(String),
}

/// What the pane shows of the selected node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodePane {
    /// Grows at every selection, Refresh, context switch, and when the filter hides the selected node.
    pub generation: u64,
    /// The selected node; `None` leaves the pane empty.
    pub node: Option<NodeId>,
    /// Its read.
    pub read: NodeRead,
}

/// What a place of the history keeps of the Nodes page: its filter and the selected node, whose pane a return reads
/// fresh.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodePlace {
    /// The id filter.
    pub prefix: String,
    /// The selected node.
    pub node: Option<NodeId>,
}

/// The Nodes page's reads, publishing a [`NodesView`] and a [`NodePane`]. Owned by the session, which hands it the
/// client of the context in use.
pub struct Nodes {
    shared: Arc<Shared>,
}

struct Shared {
    handle: Handle,
    limits: Limits,
    list: watch::Sender<NodesView>,
    pane: watch::Sender<NodePane>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    client: Option<ActorClient>,
    /// True while the page is shown.
    visible: bool,
    /// True when the cache needs a read at the next visit: not read for this client yet.
    stale: bool,
    /// Every node the last read gave.
    cache: Vec<Arc<NodeView>>,
    list_generation: u64,
    list_task: Option<JoinHandle<()>>,
    pane_generation: u64,
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

    fn supersede_list(&mut self) {
        self.list_generation += 1;
        let task = self.list_task.take();
        self.abort(task);
    }

    fn supersede_pane(&mut self) {
        self.pane_generation += 1;
        let task = self.pane_task.take();
        self.abort(task);
    }
}

impl Nodes {
    /// No client yet; reads run on `handle` within `limits.deadline`.
    pub fn new(handle: Handle, limits: Limits) -> Self {
        let shared = Shared {
            handle,
            limits,
            list: watch::channel(NodesView::default()).0,
            pane: watch::channel(NodePane::default()).0,
            state: Mutex::default(),
        };
        Self { shared: Arc::new(shared) }
    }

    /// The list's views, newest last.
    pub fn list(&self) -> watch::Receiver<NodesView> {
        self.shared.list.subscribe()
    }

    /// The pane's views, newest last.
    pub fn pane(&self) -> watch::Receiver<NodePane> {
        self.shared.pane.subscribe()
    }

    /// A context switch: drops every read, the cache and the selection; reads the list again at once when the page
    /// is visible, else at its next [`Self::show`].
    pub fn use_client(&self, client: Option<ActorClient>) {
        let mut state = self.shared.lock();
        state.supersede_list();
        state.client = client;
        state.stale = true;
        state.cache.clear();
        let (generation, prefix) = (state.list_generation, self.shared.list.borrow().prefix.clone());
        self.shared.list.send_replace(NodesView { prefix, generation, ..NodesView::default() });
        self.shared.select(&mut state, None);
        if state.visible {
            self.shared.read(&mut state);
        }
    }

    /// The page is shown: reads the list when it was not read for this client yet.
    pub fn show(&self) {
        let mut state = self.shared.lock();
        state.visible = true;
        if state.stale {
            self.shared.read(&mut state);
        }
    }

    /// Another page is shown: a context switch then waits for the next visit to read.
    pub fn hide(&self) {
        self.shared.lock().visible = false;
    }

    /// Refresh: reads the list again; the selection is cleared, as its node may be gone.
    pub fn refresh(&self) {
        let mut state = self.shared.lock();
        if state.client.is_none() {
            return;
        }
        self.shared.select(&mut state, None);
        self.shared.read(&mut state);
    }

    /// Asks again for the list after its read failed.
    pub fn retry(&self) {
        let mut state = self.shared.lock();
        if matches!(self.shared.list.borrow().read, NodesRead::Failed(_)) {
            self.shared.read(&mut state);
        }
    }

    /// A key press in the search field: filters the cache by id prefix, with no read. A selected node the filter
    /// hides is unselected, and its read dropped.
    pub fn search(&self, prefix: &str) {
        let mut state = self.shared.lock();
        self.shared.filter(&mut state, prefix);
    }

    /// Selects the node `id` and reads it under a new pane generation; the read of a node before is dropped. `None`
    /// empties the pane.
    pub fn select(&self, id: Option<&NodeId>) {
        let mut state = self.shared.lock();
        self.shared.select(&mut state, id.cloned());
    }

    /// The selected node.
    pub fn selected(&self) -> Option<NodeId> {
        self.shared.pane.borrow().node.clone()
    }

    /// Reads the selected node again after its read failed.
    pub fn retry_node(&self) {
        let mut state = self.shared.lock();
        let failed = {
            let pane = self.shared.pane.borrow();
            pane.node.clone().filter(|_| matches!(pane.read, NodeRead::Failed(_)))
        };
        if failed.is_some() {
            self.shared.select(&mut state, failed);
        }
    }

    /// The filter and the selected node, for the history to restore later.
    pub fn source(&self) -> NodePlace {
        NodePlace { prefix: self.shared.list.borrow().prefix.clone(), node: self.selected() }
    }

    /// Restores a place of the history: its filter, and its node selected and read fresh.
    pub fn restore(&self, place: NodePlace) {
        let mut state = self.shared.lock();
        self.shared.filter(&mut state, &place.prefix);
        self.shared.select(&mut state, place.node);
    }

    /// The window closes: every read is dropped.
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
}

impl Drop for Nodes {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("inspector nodes")
    }

    /// Reads every node under a new list generation, within the read deadline; the rows go at once. A result that
    /// lands after a later read or a context switch is dropped.
    fn read(self: &Arc<Self>, state: &mut State) {
        let Some(client) = state.client.clone() else { return };
        state.supersede_list();
        state.stale = false;
        state.cache.clear();
        let generation = state.list_generation;
        self.list.send_modify(|list| {
            list.generation = generation;
            list.rows.clear();
            list.loaded = 0;
            list.read = NodesRead::Loading;
        });
        let (shared, deadline) = (self.clone(), self.limits.deadline);
        state.list_task = Some(self.handle.spawn(async move {
            let answer = within(Instant::now() + deadline, client.nodes()).await;
            let mut state = shared.lock();
            if state.list_generation != generation {
                return;
            }
            state.list_task = None;
            match answer {
                Part::Ready(nodes) => {
                    state.cache = nodes.into_iter().map(Arc::new).collect();
                    let read = NodesRead::Ready(chrono::Utc::now());
                    shared.list.send_modify(|list| {
                        list.rows =
                            state.cache.iter().filter(|n| n.id.as_str().starts_with(&list.prefix)).cloned().collect();
                        list.loaded = state.cache.len();
                        list.read = read;
                    });
                }
                Part::Failed(error) => shared.list.send_modify(|list| list.read = NodesRead::Failed(error)),
            }
        }));
    }

    fn filter(self: &Arc<Self>, state: &mut State, prefix: &str) {
        self.list.send_modify(|list| {
            list.prefix = prefix.to_owned();
            list.rows = state.cache.iter().filter(|n| n.id.as_str().starts_with(prefix)).cloned().collect();
        });
        let hidden = self.pane.borrow().node.as_ref().is_some_and(|id| !id.as_str().starts_with(prefix));
        if hidden {
            self.select(state, None);
        }
    }

    /// Shows `id` in the pane under a new generation and reads it; a result that lands after the pane moved on is
    /// dropped.
    fn select(self: &Arc<Self>, state: &mut State, id: Option<NodeId>) {
        state.supersede_pane();
        let generation = state.pane_generation;
        self.pane.send_replace(NodePane { generation, node: id.clone(), read: NodeRead::Loading });
        let Some(id) = id else { return };
        let Some(client) = state.client.clone() else {
            let failed = NodeRead::Failed("No server is configured.".into());
            return self.pane.send_modify(|pane| pane.read = failed);
        };
        let (shared, deadline) = (self.clone(), self.limits.deadline);
        state.pane_task = Some(self.handle.spawn(async move {
            let answer = within(Instant::now() + deadline, client.node(&id)).await;
            let mut state = shared.lock();
            if state.pane_generation != generation {
                return;
            }
            state.pane_task = None;
            let read = match answer {
                Part::Ready(Some(node)) => NodeRead::Found(Arc::new(node)),
                Part::Ready(None) => NodeRead::NotFound,
                Part::Failed(error) => NodeRead::Failed(error),
            };
            shared.pane.send_modify(|pane| pane.read = read);
        }));
    }
}

/// A node's health as the server computed it, as on the wire.
pub fn health_text(health: NodeHealth) -> &'static str {
    match health {
        NodeHealth::Alive => "alive",
        NodeHealth::Suspect => "suspect",
        NodeHealth::Dead => "dead",
    }
}

/// The types a node hosts, or "sends only" for a node that hosts none.
pub fn hosted_text(hosted: &[String]) -> String {
    if hosted.is_empty() { "sends only".to_owned() } else { hosted.join(", ") }
}
