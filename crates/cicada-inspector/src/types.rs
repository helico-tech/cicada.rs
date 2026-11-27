//! Registered types, cached once per reload, and the selected schema formatted off the main thread.

use std::sync::{Arc, Mutex};

use cicada::api::TypeView;
use cicada_client::ActorClient;
use tokio::{runtime::Handle, sync::watch, task::JoinHandle, time::Instant};

use crate::json::{self, Json};
use crate::model::{Limits, Part, within};
use crate::pager::Paging;

/// The rows matching the Types search field.
#[derive(Clone, Default)]
pub(crate) struct TypesView {
    /// The name prefix.
    pub prefix: String,
    /// The matching types in server order, shared with the cache and the pane.
    pub rows: Vec<Arc<TypeView>>,
    /// The state of the whole-list read.
    pub paging: Paging,
}

impl TypesView {
    /// The row with this name.
    pub fn index(&self, name: &str) -> Option<usize> {
        self.rows.iter().position(|row| row.name == name)
    }
}

/// The selected type and schema.
#[derive(Clone, Default)]
pub(crate) struct TypePane {
    /// The selected type, if any.
    pub row: Option<Arc<TypeView>>,
    /// State at index zero, followed by the messages in name order.
    pub schema: usize,
    /// The formatted schema, once ready.
    pub json: Option<Arc<Json>>,
}

impl TypePane {
    /// The names in the schema pop-up, in the order of their selection indices.
    pub fn schemas(&self) -> Vec<&str> {
        self.row
            .iter()
            .flat_map(|row| std::iter::once("State").chain(row.messages.keys().map(String::as_str)))
            .collect()
    }
}

/// The Types reads and formatting for one inspector session.
pub(crate) struct Types {
    shared: Arc<Shared>,
}

struct Shared {
    handle: Handle,
    limits: Limits,
    list: watch::Sender<TypesView>,
    pane: watch::Sender<TypePane>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    client: Option<ActorClient>,
    loaded: bool,
    generation: u64,
    task: Option<JoinHandle<()>>,
    rows: Vec<Arc<TypeView>>,
    pane_generation: u64,
    pane_visible: bool,
    formatting: bool,
    #[cfg(test)]
    formatted: usize,
    #[cfg(test)]
    hold: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
}

impl State {
    fn clear(&mut self) {
        self.generation += 1;
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.rows.clear();
        self.pane_generation += 1;
    }
}

impl Types {
    /// Starts with no client or visible pane.
    pub fn new(handle: Handle, limits: Limits) -> Self {
        let shared = Shared {
            handle,
            limits,
            list: watch::channel(TypesView::default()).0,
            pane: watch::channel(TypePane::default()).0,
            state: Mutex::default(),
        };
        Self { shared: Arc::new(shared) }
    }

    /// Subscribes to the filtered list.
    pub fn list(&self) -> watch::Receiver<TypesView> {
        self.shared.list.subscribe()
    }

    /// Subscribes to the selection and its formatting.
    pub fn pane(&self) -> watch::Receiver<TypePane> {
        self.shared.pane.subscribe()
    }

    /// Clears the old context's data. The next visit reads the list.
    pub fn use_client(&self, client: Option<ActorClient>) {
        let mut state = self.shared.state.lock().expect("types");
        state.clear();
        state.client = client;
        state.loaded = false;
        self.shared.list.send_modify(|list| {
            list.rows.clear();
            list.paging = Paging::Idle;
        });
        self.shared.pane.send_replace(TypePane::default());
    }

    /// Reads the list on the first visit to this context.
    pub fn show(&self) {
        let mut state = self.shared.state.lock().expect("types");
        if !state.loaded {
            self.shared.reload(&mut state);
        }
    }

    /// Clears the list and selection and reads the list once again.
    pub fn refresh(&self) {
        self.shared.reload(&mut self.shared.state.lock().expect("types"));
    }

    /// Reads the list again after a failure.
    pub fn retry(&self) {
        let mut state = self.shared.state.lock().expect("types");
        if matches!(self.shared.list.borrow().paging, Paging::Failed(_)) {
            self.shared.reload(&mut state);
        }
    }

    /// Filters the cache and clears a selection that no longer matches.
    pub fn search(&self, prefix: &str) {
        let mut state = self.shared.state.lock().expect("types");
        self.shared.list.send_modify(|list| {
            list.prefix = prefix.to_owned();
            list.rows = matching(&state.rows, prefix);
        });
        let hidden = self.shared.pane.borrow().row.as_ref().is_some_and(|row| !row.name.starts_with(prefix));
        if hidden {
            state.pane_generation += 1;
            self.shared.pane.send_replace(TypePane::default());
        }
    }

    /// Selects a visible type and its State schema, or clears the pane.
    pub fn select(&self, name: Option<&str>) {
        let mut state = self.shared.state.lock().expect("types");
        let row = {
            let list = self.shared.list.borrow();
            name.and_then(|name| list.index(name)).map(|index| list.rows[index].clone())
        };
        state.pane_generation += 1;
        self.shared.pane.send_replace(TypePane { row, ..TypePane::default() });
        self.shared.format_if_needed(&mut state);
    }

    /// Chooses a schema by its index in the pop-up.
    pub fn choose(&self, schema: usize) {
        let mut state = self.shared.state.lock().expect("types");
        let pane = self.shared.pane.borrow().clone();
        if schema != pane.schema && pane.row.as_ref().is_some_and(|row| schema <= row.messages.len()) {
            state.pane_generation += 1;
            self.shared.pane.send_replace(TypePane { schema, json: None, ..pane });
            self.shared.format_if_needed(&mut state);
        }
    }

    /// Starts any pending formatting when the pane is shown.
    pub fn set_pane_visible(&self, visible: bool) {
        let mut state = self.shared.state.lock().expect("types");
        state.pane_visible = visible;
        self.shared.format_if_needed(&mut state);
    }

    /// Cancels reads and drops results for a closed window.
    pub fn stop(&self) {
        self.use_client(None);
    }
}

impl Shared {
    fn reload(self: &Arc<Self>, state: &mut State) {
        let Some(client) = state.client.clone() else { return };
        state.clear();
        state.loaded = true;
        self.list.send_modify(|list| {
            list.rows.clear();
            list.paging = Paging::Loading;
        });
        self.pane.send_replace(TypePane::default());
        let generation = state.generation;
        let shared = self.clone();
        state.task = Some(self.handle.spawn(async move {
            let answer = within(Instant::now() + shared.limits.deadline, client.types()).await;
            let mut state = shared.state.lock().expect("types");
            if state.generation != generation {
                return;
            }
            state.task = None;
            shared.list.send_modify(|list| match answer {
                Part::Ready(rows) => {
                    state.rows = rows.into_iter().map(Arc::new).collect();
                    list.rows = matching(&state.rows, &list.prefix);
                    list.paging = Paging::Idle;
                }
                Part::Failed(error) => list.paging = Paging::Failed(error),
            });
        }));
    }

    fn format_if_needed(self: &Arc<Self>, state: &mut State) {
        if state.formatting || !state.pane_visible {
            return;
        }
        let (row, schema) = {
            let pane = self.pane.borrow();
            let Some(row) = pane.row.clone().filter(|_| pane.json.is_none()) else { return };
            (row, pane.schema)
        };
        state.formatting = true;
        let generation = state.pane_generation;
        #[cfg(test)]
        let hold = state.hold.take();
        let shared = self.clone();
        self.handle.spawn_blocking(move || {
            #[cfg(test)]
            if let Some((arrived, release)) = hold {
                let _ = arrived.send(());
                let _ = release.recv_timeout(std::time::Duration::from_secs(10));
            }
            let value = if schema == 0 { &row.state } else { row.messages.values().nth(schema - 1).unwrap() };
            let json = Arc::new(json::format(value));
            let mut state = shared.state.lock().expect("types");
            state.formatting = false;
            #[cfg(test)]
            {
                state.formatted += 1;
            }
            if state.pane_generation == generation {
                shared.pane.send_modify(|pane| pane.json = Some(json));
            } else {
                shared.format_if_needed(&mut state);
            }
        });
    }
}

fn matching(rows: &[Arc<TypeView>], prefix: &str) -> Vec<Arc<TypeView>> {
    rows.iter().filter(|row| row.name.starts_with(prefix)).cloned().collect()
}

#[cfg(test)]
impl Types {
    /// Counts completed formattings, including ones dropped before publication.
    pub fn formatted(&self) -> usize {
        self.shared.state.lock().expect("types").formatted
    }

    /// Holds the next formatting until the test releases it.
    pub fn hold_format(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (arrived, arrival) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        self.shared.state.lock().expect("types").hold = Some((arrived, released));
        (arrival, release)
    }
}
