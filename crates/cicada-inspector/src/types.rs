//! Registered types, cached once per reload, and the selected schema formatted off the main thread.

use std::sync::{Arc, Mutex, MutexGuard};

use cicada::api::TypeView;
use cicada_client::ActorClient;
use tokio::{runtime::Handle, sync::watch, task::JoinHandle, time::Instant};

use crate::json::{self, Json};
use crate::model::{Limits, Part, within};
use crate::pager::Paging;
use crate::refresh::{Budget, Done, Readiness, Refresh, RefreshPage, Stale, as_of};

/// The rows matching the Types search field.
#[derive(Clone, Default)]
pub(crate) struct TypesView {
    /// The name prefix.
    pub prefix: String,
    /// The matching types in server order, shared with the cache and the pane.
    pub rows: Vec<Arc<TypeView>>,
    /// The state of the whole-list read.
    pub paging: Paging,
    /// The mark on a list a soft refresh could not update.
    pub stale: Option<Stale>,
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
    refresh: Refresh,
    read_at: Option<cicada::time::Timestamp>,
    rows: Vec<Arc<TypeView>>,
    pane_generation: u64,
    pane_visible: bool,
    formatting: bool,
    formatted_generation: Option<u64>,
    #[cfg(test)]
    formatted: usize,
    #[cfg(test)]
    hold: std::collections::VecDeque<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
}

impl State {
    fn clear(&mut self) {
        self.generation += 1;
        self.refresh.cancel();
        self.read_at = None;
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
            list.stale = None;
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

    /// Re-reads the registered types and the selected schema without clearing them; [`Done`] when it ends.
    pub fn soft_refresh(&self) -> Done {
        self.shared.request()
    }

    /// Cancels a soft refresh when this page is left.
    pub fn cancel_soft_refresh(&self) {
        self.shared.lock_refresh().refresh.cancel();
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
        if self.shared.list.borrow().prefix != prefix {
            state.refresh.cancel();
        }
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
            list.stale = None;
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
            match answer {
                Part::Ready(rows) => shared.replace_types(&mut state, rows),
                Part::Failed(error) => shared.list.send_modify(|list| list.paging = Paging::Failed(error)),
            }
            shared.resume(&mut state);
        }));
    }

    fn replace_types(&self, state: &mut State, rows: Vec<TypeView>) {
        state.read_at = Some(chrono::Utc::now());
        state.rows = rows.into_iter().map(Arc::new).collect();
        self.list.send_modify(|list| {
            list.rows = matching(&state.rows, &list.prefix);
            list.paging = Paging::Idle;
            list.stale = None;
        });
    }

    fn format_if_needed(self: &Arc<Self>, state: &mut State) {
        if state.formatting || !state.pane_visible || state.formatted_generation == Some(state.pane_generation) {
            return;
        }
        let (row, schema) = {
            let pane = self.pane.borrow();
            let Some(row) = pane.row.clone() else { return };
            (row, pane.schema)
        };
        state.formatting = true;
        let generation = state.pane_generation;
        let format = self.formatter(state, row, schema);
        let shared = self.clone();
        self.handle.spawn_blocking(move || {
            let json = format();
            let mut state = shared.state.lock().expect("types");
            state.formatting = false;
            if state.pane_generation == generation {
                state.formatted_generation = Some(generation);
                shared.pane.send_modify(|pane| pane.json = Some(json));
            } else {
                shared.format_if_needed(&mut state);
            }
        });
    }

    /// Formats the schema `schema` of `row` as highlighted JSON, for a blocking thread; tests can hold it and count it.
    fn formatter(
        self: &Arc<Self>,
        state: &mut State,
        row: Arc<TypeView>,
        schema: usize,
    ) -> impl FnOnce() -> Arc<Json> + Send + use<> {
        #[cfg(test)]
        let (hold, shared) = (state.hold.pop_front(), self.clone());
        #[cfg(not(test))]
        let _ = (state, self);
        move || {
            #[cfg(test)]
            if let Some((arrived, release)) = hold {
                let _ = arrived.send(());
                let _ = release.recv_timeout(std::time::Duration::from_secs(10));
            }
            let value = if schema == 0 { &row.state } else { row.messages.values().nth(schema - 1).unwrap() };
            let json = Arc::new(json::format(value));
            #[cfg(test)]
            {
                shared.state.lock().expect("types").formatted += 1;
            }
            json
        }
    }
}

impl RefreshPage for Shared {
    type State = State;

    fn lock_refresh(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("types")
    }

    fn refresh(state: &mut State) -> &mut Refresh {
        &mut state.refresh
    }

    fn runtime(&self) -> (&Handle, std::time::Duration) {
        (&self.handle, self.limits.deadline)
    }

    fn readiness(&self, state: &State) -> Readiness {
        match self.list.borrow().paging {
            Paging::Loading => Readiness::Wait,
            Paging::Idle if state.loaded && state.client.is_some() => Readiness::Ready,
            _ => Readiness::Skip,
        }
    }

    async fn run(self: &Arc<Self>, ticket: u64, budget: Budget) {
        let client = {
            let state = self.lock_refresh();
            if !state.refresh.current(ticket) {
                return;
            }
            let Some(client) = state.client.clone() else { return };
            client
        };
        let answer = budget.read(client.types()).await;
        let Some((generation, pane, format)) = self.land_types(&mut self.lock_refresh(), ticket, answer) else {
            return;
        };
        // The pane keeps showing the type as it was, its schema menu included, until the new one is formatted; then
        // the type, its menu and its JSON show together, only while this refresh and the pane are still current.
        let Ok(json) = self.handle.spawn_blocking(format).await else { return };
        let mut state = self.lock_refresh();
        if state.refresh.current(ticket) && state.pane_generation == generation {
            state.pane_generation += 1;
            state.formatted_generation = Some(state.pane_generation);
            self.pane.send_replace(TypePane { json: Some(json), ..pane });
        }
    }
}

/// A soft refresh's new pane for the selected type: the pane generation it replaces, the pane without its JSON, and
/// the formatting of that JSON.
type RefreshedPane = (u64, TypePane, Box<dyn FnOnce() -> Arc<Json> + Send>);

impl Shared {
    /// Lands a soft refresh's types and keeps the pane's type and schema by name. A selected type that changed is
    /// returned for the refresh to format and publish; one that is gone empties the pane at once.
    fn land_types(
        self: &Arc<Self>,
        state: &mut State,
        ticket: u64,
        answer: Part<Vec<TypeView>>,
    ) -> Option<RefreshedPane> {
        if !state.refresh.current(ticket) {
            return None;
        }
        let rows = match answer {
            Part::Ready(rows) => rows,
            Part::Failed(error) => {
                self.list.send_modify(|list| {
                    list.stale = Some(Stale::failed(as_of(list.stale.as_ref(), state.read_at), error));
                });
                return None;
            }
        };
        self.replace_types(state, rows);
        let before = self.pane.borrow().clone();
        let previous = before.row?;
        let row = self.list.borrow().rows.iter().find(|row| row.name == previous.name).cloned();
        if row.as_ref() == Some(&previous) {
            return None;
        }
        // The schema popup lists message names in order, so an updated contract can move the selected schema.
        let message = before.schema.checked_sub(1).and_then(|index| previous.messages.keys().nth(index));
        let schema = row
            .as_ref()
            .zip(message)
            .and_then(|(row, name)| row.messages.keys().position(|key| key == name))
            .map_or(0, |index| index + 1);
        let Some(row) = row else {
            state.pane_generation += 1;
            self.pane.send_replace(TypePane::default());
            return None;
        };
        let format = Box::new(self.formatter(state, row.clone(), schema));
        Some((state.pane_generation, TypePane { row: Some(row), schema, json: None }, format))
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

    /// Holds the next formatting not yet held until the test releases it.
    pub fn hold_format(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (arrived, arrival) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        self.shared.state.lock().expect("types").hold.push_back((arrived, released));
        (arrival, release)
    }
}
