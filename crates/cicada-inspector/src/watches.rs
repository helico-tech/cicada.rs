//! Stored watches, paged in the server's order and filtered over the rows already loaded.

use std::sync::{Arc, Mutex, MutexGuard};

use cicada::api::WatchView;
use cicada::{ActorAddress, WatchTarget};
use cicada_client::ActorClient;
use tokio::{runtime::Handle, sync::watch};

use crate::model::{Limits, Part};
use crate::pager::{Landed, NEAR_END, PAGE, Paged, Pager, Paging, SoftPaged};
use crate::refresh::{Done, RefreshPage, Stale};

/// The locally filtered table and the state of the server's paging.
#[derive(Clone, Default)]
pub(crate) struct WatchesView {
    /// The watcher or target prefix.
    pub prefix: String,
    /// Matching watches in server order, shared with the cache and the pane.
    pub rows: Vec<Arc<WatchView>>,
    /// The number of server rows loaded, including those outside the filter.
    pub loaded: usize,
    /// True when the server has another page.
    pub more: bool,
    /// Whether a page is pending or failed.
    pub paging: Paging,
    /// The mark on rows carried over by a soft refresh.
    pub stale: Option<Stale>,
}

impl WatchesView {
    /// The row of the exact watch.
    pub fn index(&self, watch: &WatchView) -> Option<usize> {
        self.rows.iter().position(|row| **row == *watch)
    }

    /// True when scrolling should load another page.
    pub fn wants_more(&self, last: usize) -> bool {
        self.prefix.is_empty()
            && self.more
            && self.paging == Paging::Idle
            && last.saturating_add(NEAR_END) >= self.loaded
    }
}

/// The state of the Watches page to restore when returning through the history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WatchPlace {
    prefix: String,
    selected: Option<Arc<WatchView>>,
}

/// The Watches pages and selection for one inspector session.
pub(crate) struct Watches {
    shared: Arc<Shared>,
}

struct Shared {
    handle: Handle,
    limits: Limits,
    list: watch::Sender<WatchesView>,
    pane: watch::Sender<Option<Arc<WatchView>>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    client: Option<ActorClient>,
    visited: bool,
    pager: Pager,
    rows: Vec<Arc<WatchView>>,
    /// A history selection restored when the page holding the watch arrives.
    pending: Option<Arc<WatchView>>,
}

impl Watches {
    /// Starts with no client and no stored rows.
    pub fn new(handle: Handle, limits: Limits) -> Self {
        let shared = Shared {
            handle,
            limits,
            list: watch::channel(WatchesView::default()).0,
            pane: watch::channel(None).0,
            state: Mutex::default(),
        };
        Self { shared: Arc::new(shared) }
    }

    /// Subscribes to the visible watches.
    pub fn list(&self) -> watch::Receiver<WatchesView> {
        self.shared.list.subscribe()
    }

    /// Subscribes to the selected watch.
    pub fn pane(&self) -> watch::Receiver<Option<Arc<WatchView>>> {
        self.shared.pane.subscribe()
    }

    /// Clears the old context's data. The next visit reads from the top.
    pub fn use_client(&self, client: Option<ActorClient>) {
        let mut state = self.shared.lock();
        self.shared.clear(&mut state);
        state.client = client;
        state.visited = false;
    }

    /// Reads the first page on the first visit to this context, even under a filter.
    pub fn show(&self) {
        let mut state = self.shared.lock();
        if !state.visited {
            self.shared.request_page(&mut state);
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

    /// Clears the selection and starts paging from the top again.
    pub fn refresh(&self) {
        let mut state = self.shared.lock();
        self.shared.clear(&mut state);
        self.shared.request_page(&mut state);
    }

    /// Filters the loaded rows without a read, clearing a selection outside the filter.
    pub fn search(&self, prefix: &str) {
        let mut state = self.shared.lock();
        state.pending = None;
        state.pager.cancel_refresh();
        self.shared.filter(&state, prefix);
    }

    /// Loads a page in response to scrolling only if the current filter and row allow it.
    pub fn scroll(&self, last: usize) {
        let mut state = self.shared.lock();
        if self.shared.list.borrow().wants_more(last) {
            self.shared.request_page(&mut state);
        }
    }

    /// Loads one page in response to a Load more click.
    pub fn more(&self) {
        let mut state = self.shared.lock();
        let ask = {
            let list = self.shared.list.borrow();
            list.more && list.paging == Paging::Idle
        };
        if ask {
            self.shared.request_page(&mut state);
        }
    }

    /// Reads the failed page again from the same cursor.
    pub fn retry(&self) {
        let mut state = self.shared.lock();
        if matches!(self.shared.list.borrow().paging, Paging::Failed(_)) {
            self.shared.request_page(&mut state);
        }
    }

    /// Selects a visible watch or clears the selection.
    pub fn select(&self, watch: Option<&WatchView>) {
        let mut state = self.shared.lock();
        state.pending = None;
        let row = self.shared.find(watch);
        self.shared.pane.send_replace(row);
    }

    /// The selected watch's watcher or, when it names one actor, its target.
    pub fn actor(&self, target: bool) -> Option<ActorAddress> {
        let pane = self.shared.pane.borrow();
        let watch = pane.as_ref()?;
        if target {
            match &watch.target {
                WatchTarget::Actor(actor) => Some(actor.clone()),
                _ => None,
            }
        } else {
            Some(watch.watcher.clone())
        }
    }

    /// The current filter and selection for Back and Forward.
    pub fn source(&self) -> WatchPlace {
        let state = self.shared.lock();
        WatchPlace {
            prefix: self.shared.list.borrow().prefix.clone(),
            selected: self.shared.pane.borrow().clone().or_else(|| state.pending.clone()),
        }
    }

    /// Restores a history place's filter and selection, waiting for the row if it is on an unread page.
    pub fn restore(&self, place: WatchPlace) {
        let mut state = self.shared.lock();
        if self.shared.list.borrow().prefix != place.prefix {
            state.pager.cancel_refresh();
        }
        self.shared.filter(&state, &place.prefix);
        let row = self.shared.find(place.selected.as_deref());
        let can_land = {
            let list = self.shared.list.borrow();
            list.more || list.paging != Paging::Idle
        };
        state.pending = if row.is_none() && can_land { place.selected } else { None };
        self.shared.pane.send_replace(row);
    }

    /// Cancels reads and drops their results for a closed window.
    pub fn stop(&self) {
        self.use_client(None);
    }
}

impl Shared {
    fn clear(&self, state: &mut State) {
        for task in state.pager.restart().into_iter().flatten() {
            task.abort();
        }
        state.rows.clear();
        state.pending = None;
        self.list.send_modify(|list| {
            list.stale = None;
            list.rows.clear();
            list.loaded = 0;
            list.more = false;
            list.paging = Paging::Idle;
        });
        self.pane.send_replace(None);
    }

    fn filter(&self, state: &State, prefix: &str) {
        self.list.send_modify(|list| {
            list.prefix = prefix.to_owned();
            list.rows = state.rows.iter().filter(|row| matches_prefix(row, prefix)).cloned().collect();
        });
        let hidden = self.pane.borrow().as_ref().is_some_and(|row| !matches_prefix(row, prefix));
        if hidden {
            self.pane.send_replace(None);
        }
    }

    fn find(&self, watch: Option<&WatchView>) -> Option<Arc<WatchView>> {
        let list = self.list.borrow();
        watch.and_then(|watch| list.index(watch)).map(|index| list.rows[index].clone())
    }

    fn request_page(self: &Arc<Self>, state: &mut State) {
        let Some(client) = state.client.clone() else { return };
        let query = ();
        let read = move |cursor| Self::read_page(client, query, cursor);
        if Pager::request(self, state, self.limits, read, Self::land) {
            state.visited = true;
            self.list.send_modify(|list| list.paging = Paging::Loading);
        }
    }

    fn land(self: &Arc<Self>, state: &mut State, landed: Landed<WatchView>) {
        match landed.page {
            Part::Ready((rows, more)) => {
                let added = crate::pager::insert_ordered(&mut state.rows, rows, Self::key);
                self.list.send_modify(|list| {
                    list.rows = state.rows.iter().filter(|row| matches_prefix(row, &list.prefix)).cloned().collect();
                    list.loaded = state.rows.len();
                    list.more = more;
                    list.paging = Paging::Idle;
                });
                if let Some(row) = self.find(state.pending.as_deref()) {
                    state.pending = None;
                    self.pane.send_replace(Some(row));
                } else if !more {
                    state.pending = None;
                }
                if added == 0 && more {
                    self.request_page(state);
                }
            }
            Part::Failed(error) => self.list.send_modify(|list| list.paging = Paging::Failed(error)),
        }
        self.resume(state);
    }
}

impl Paged for Shared {
    type State = State;

    fn handle(&self) -> &Handle {
        &self.handle
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("watches")
    }

    fn pager(state: &mut State) -> &mut Pager {
        &mut state.pager
    }
}

fn matches_prefix(watch: &WatchView, prefix: &str) -> bool {
    watch.watcher.as_str().starts_with(prefix) || watch.target.as_str().starts_with(prefix)
}

impl SoftPaged for Shared {
    type Row = WatchView;
    type Key = (String, ActorAddress);
    type Sort = Self::Key;
    type Query = ();

    fn key(row: &Self::Row) -> Self::Key {
        (row.target.as_str().to_owned(), row.watcher.clone())
    }

    fn sort(row: &Self::Row) -> Self::Sort {
        Self::key(row)
    }

    fn rows(&self, state: &State) -> Vec<Arc<Self::Row>> {
        state.rows.clone()
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
        Some((state.client.clone().filter(|_| state.visited)?, ()))
    }

    async fn read_page(
        client: ActorClient,
        _query: Self::Query,
        cursor: Option<String>,
    ) -> Result<cicada::api::PageView<Self::Row>, cicada_client::ClientError> {
        client.watches(None, None, cursor.as_deref(), Some(PAGE)).await
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
        state.rows = rows;
        self.list.send_modify(|list| {
            list.rows = state.rows.iter().filter(|row| matches_prefix(row, &list.prefix)).cloned().collect();
            list.loaded = state.rows.len();
            list.more = more;
            list.stale = stale;
        });
        let selected = self.pane.borrow().clone();
        let pending = self.find(state.pending.as_deref());
        if pending.is_some() || !more {
            state.pending = None;
        }
        let row = pending.or_else(|| self.find(selected.as_deref()));
        self.pane.send_replace(row);
    }

    fn page(self: &Arc<Self>, state: &mut State) {
        self.request_page(state);
    }
}
