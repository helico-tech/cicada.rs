//! The Running Turns page: the turns running now under a prefix, read a page at a time through the root pager; and the
//! pane of the selected turn, filled from its row. The server pages turns by an exclusive actor-address cursor, so an
//! actor never shows on two pages and the rows are not de-duplicated. A place of the history keeps the turn as read;
//! a return to it reads the actor's turn fresh by its exact address, to tell a turn running now from one that ended.
//! The list's generation grows at every key press in the search field, on Refresh and on a context switch; the pane's
//! at every selection, restore, Refresh and context switch.

use std::sync::{Arc, Mutex, MutexGuard};

use cicada::ActorAddress;
use cicada::api::RunningTurnView;
use cicada_client::ActorClient;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::model::{Limits, Part, within};
use crate::pager::{Landed, NEAR_END, PAGE, Paged, Pager, Paging};

/// What the Running Turns table shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnsView {
    /// The prefix of the actor addresses the rows are under.
    pub prefix: String,
    /// Grows whenever the rows are replaced rather than appended to.
    pub generation: u64,
    /// The turns read so far, in the server's order; shared with the pane and with later views.
    pub rows: Vec<Arc<RunningTurnView>>,
    /// Whether a page is asked for or failed.
    pub paging: Paging,
    /// True when the server has rows past these.
    pub more: bool,
}

impl TurnsView {
    /// True when the view should ask for the next page with `last` the last visible row: it is within [`NEAR_END`]
    /// of the end, the server has more, and no page is asked for or failed.
    pub fn wants_more(&self, last: usize) -> bool {
        self.more && self.paging == Paging::Idle && last + NEAR_END >= self.rows.len()
    }

    /// The row of the turn of `actor`.
    pub fn index(&self, actor: &ActorAddress) -> Option<usize> {
        self.rows.iter().position(|row| &row.actor == actor)
    }

    /// The row the table selects for `pane`, from the latest list and pane, whichever changed last.
    pub fn selection(&self, pane: &TurnPane) -> Option<usize> {
        pane.selected.as_ref().and_then(|actor| self.index(actor))
    }
}

/// Where the pane's turn comes from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TurnState {
    /// The turn runs: from its row, or from a fresh read of its actor's turn.
    #[default]
    Running,
    /// A return to the place reads the actor's turn again; the pane shows the turn as it was read meanwhile.
    Checking,
    /// The actor runs no turn now: the pane shows the turn as it was read, "no longer running".
    Ended,
    /// The fresh read failed: the text to show; Retry reads the actor's turn again.
    Failed(String),
}

/// What the pane shows of the selected turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnPane {
    /// Grows at every selection, restore, Refresh and context switch.
    pub generation: u64,
    /// The turn shown; `None` leaves the pane empty.
    pub turn: Option<Arc<RunningTurnView>>,
    /// Whether it still runs.
    pub state: TurnState,
    /// The actor whose row the table selects: a row the user picked, or the turn a return found running once its row
    /// is loaded. Published with the pane whichever comes first, the fresh read or the page with the row.
    pub selected: Option<ActorAddress>,
}

/// What a place of the history keeps of the Running Turns page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnPlace {
    /// The search prefix.
    pub prefix: String,
    /// The turn the pane showed, as it was read.
    pub turn: Option<Arc<RunningTurnView>>,
}

/// The Running Turns page's reads, publishing a [`TurnsView`] and a [`TurnPane`]. Owned by the session, which hands
/// it the client of the context in use.
pub struct RunningTurns {
    shared: Arc<Shared>,
}

struct Shared {
    handle: Handle,
    limits: Limits,
    list: watch::Sender<TurnsView>,
    pane: watch::Sender<TurnPane>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    client: Option<ActorClient>,
    /// True once the list was read for this client; [`RunningTurns::show`] reads it only the first time.
    loaded: bool,
    pager: Pager,
    prefix: String,
    pane_generation: u64,
    /// An actor whose row is selected once a page of this generation brings it: a return found its turn running.
    pending: Option<ActorAddress>,
    /// The fresh read of a restored turn's actor.
    check_task: Option<JoinHandle<()>>,
}

impl State {
    fn abort(&mut self, task: Option<JoinHandle<()>>) {
        if let Some(task) = task {
            task.abort();
        }
    }

    fn supersede_list(&mut self) {
        let [page, debounce] = self.pager.restart();
        self.abort(page);
        self.abort(debounce);
    }

    fn supersede_pane(&mut self) {
        self.pane_generation += 1;
        self.pending = None;
        let task = self.check_task.take();
        self.abort(task);
    }
}

impl RunningTurns {
    /// No client yet; reads run on `handle` within `limits.deadline`, and a search waits `limits.debounce`.
    pub fn new(handle: Handle, limits: Limits) -> Self {
        let shared = Shared {
            handle,
            limits,
            list: watch::channel(TurnsView::default()).0,
            pane: watch::channel(TurnPane::default()).0,
            state: Mutex::default(),
        };
        Self { shared: Arc::new(shared) }
    }

    /// The list's views, newest last.
    pub fn list(&self) -> watch::Receiver<TurnsView> {
        self.shared.list.subscribe()
    }

    /// The pane's views, newest last.
    pub fn pane(&self) -> watch::Receiver<TurnPane> {
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
        self.shared.list.send_replace(TurnsView { prefix, generation, ..TurnsView::default() });
        self.shared.show(&mut state, None, TurnState::Running, None);
    }

    /// The page is shown: reads the list the first time for this client.
    pub fn show(&self) {
        let mut state = self.shared.lock();
        if !state.loaded && state.client.is_some() {
            self.shared.reload(&mut state, false);
        }
    }

    /// Refresh: reads the list again from the top under its prefix; the selection is cleared, as its turn may have
    /// ended.
    pub fn refresh(&self) {
        let mut state = self.shared.lock();
        if state.client.is_none() {
            return;
        }
        self.shared.show(&mut state, None, TurnState::Running, None);
        self.shared.reload(&mut state, false);
    }

    /// A key press in the search field: the rows go at once, and the new prefix is read once the field has been
    /// still for `limits.debounce`. A turn of an actor outside the prefix leaves the pane.
    pub fn search(&self, prefix: &str) {
        let mut state = self.shared.lock();
        state.prefix = prefix.to_owned();
        let outside = self.shared.pane.borrow().turn.as_ref().is_some_and(|t| !t.actor.as_str().starts_with(prefix));
        if outside {
            self.shared.show(&mut state, None, TurnState::Running, None);
        }
        state.pending = None;
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

    /// Selects the turn of `actor` among the rows and fills the pane from its row, with no read. `None`, or an actor
    /// with no row, empties the pane.
    pub fn select(&self, actor: Option<&ActorAddress>) {
        let mut state = self.shared.lock();
        let row = actor.and_then(|actor| {
            let list = self.shared.list.borrow();
            list.index(actor).map(|index| list.rows[index].clone())
        });
        let selected = row.as_ref().map(|row| row.actor.clone());
        self.shared.show(&mut state, row, TurnState::Running, selected);
    }

    /// The actor of the selected row.
    #[cfg(test)]
    pub fn selected(&self) -> Option<ActorAddress> {
        self.shared.pane.borrow().selected.clone()
    }

    /// The prefix and the turn the pane shows, as read, for the history to restore later.
    pub fn source(&self) -> TurnPlace {
        let state = self.shared.lock();
        TurnPlace { prefix: state.prefix.clone(), turn: self.shared.pane.borrow().turn.clone() }
    }

    /// Restores a place of the history: reads its prefix again at once when it is not the current one, and reads its
    /// turn's actor fresh by its exact address. While that read is out the pane shows the turn as it was read; then
    /// the turn running now, with its row selected when it is loaded or once a page brings it, or the turn as read
    /// marked "no longer running".
    pub fn restore(&self, place: TurnPlace) {
        let mut state = self.shared.lock();
        if state.prefix != place.prefix {
            state.prefix = place.prefix.clone();
            self.shared.show(&mut state, None, TurnState::Running, None);
            if state.client.is_none() {
                self.shared.list.send_modify(|list| list.prefix = place.prefix.clone());
            } else {
                self.shared.reload(&mut state, false);
            }
        }
        match place.turn {
            None => self.shared.show(&mut state, None, TurnState::Running, None),
            Some(turn) => {
                self.shared.show(&mut state, Some(turn.clone()), TurnState::Checking, None);
                self.shared.check(&mut state, turn);
            }
        }
    }

    /// Reads the restored turn's actor again after that read failed.
    pub fn retry_check(&self) {
        let mut state = self.shared.lock();
        let failed = {
            let pane = self.shared.pane.borrow();
            pane.turn.clone().filter(|_| matches!(pane.state, TurnState::Failed(_)))
        };
        if let Some(turn) = failed {
            self.shared.show(&mut state, Some(turn.clone()), TurnState::Checking, None);
            self.shared.check(&mut state, turn);
        }
    }

    /// The window closes: every read is dropped.
    pub fn stop(&self) {
        let mut state = self.shared.lock();
        state.supersede_list();
        state.supersede_pane();
    }
}

impl Drop for RunningTurns {
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
        self.state.lock().expect("inspector turns")
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
        self.list.send_replace(TurnsView { prefix, generation, paging: Paging::Loading, more: true, rows: vec![] });
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
            client.turns(None, &prefix, cursor.as_deref(), Some(PAGE)).await
        };
        if Pager::request(self, state, self.limits, read, Self::land_page) {
            self.list.send_modify(|list| list.paging = Paging::Loading);
        }
    }

    /// Appends the turns of a page. A page with no rows asks for the next at once while the server has more: the rows
    /// the view sees did not change, so it would not ask again.
    fn land_page(self: &Arc<Self>, state: &mut State, landed: Landed<RunningTurnView>) {
        let (items, more) = match landed.page {
            Part::Ready(page) => page,
            Part::Failed(error) => return self.list.send_modify(|list| list.paging = Paging::Failed(error)),
        };
        let empty = items.is_empty();
        let rows: Vec<_> = items.into_iter().map(Arc::new).collect();
        // The row of a turn a return found running arrived: it is selected, and the pane keeps the turn as read.
        let arrived = state.pending.take_if(|actor| rows.iter().any(|row| &row.actor == actor));
        if arrived.is_none() && !more {
            state.pending = None;
        }
        self.list.send_modify(|list| {
            list.rows.extend(rows);
            list.paging = Paging::Idle;
            list.more = more;
        });
        if let Some(actor) = arrived {
            self.pane.send_modify(|pane| pane.selected = Some(actor));
        }
        if empty && more {
            self.request_page(state);
        }
    }

    /// Shows `turn` in the pane under a new generation, with `selected` the row selected; the read of a turn before
    /// is dropped.
    fn show(
        &self,
        state: &mut State,
        turn: Option<Arc<RunningTurnView>>,
        turn_state: TurnState,
        selected: Option<ActorAddress>,
    ) {
        state.supersede_pane();
        let generation = state.pane_generation;
        self.pane.send_replace(TurnPane { generation, turn, state: turn_state, selected });
    }

    /// Reads the turn of `saved`'s actor by its exact address, under the pane's generation: a turn running now is
    /// shown as it is, with its row selected when loaded or once a page brings it; none shows `saved` as ended.
    fn check(self: &Arc<Self>, state: &mut State, saved: Arc<RunningTurnView>) {
        let Some(client) = state.client.clone() else {
            let failed = TurnState::Failed("No server is configured.".into());
            return self.pane.send_modify(|pane| pane.state = failed);
        };
        let (generation, shared, deadline) = (state.pane_generation, self.clone(), self.limits.deadline);
        state.check_task = Some(self.handle.spawn(async move {
            let answer = within(Instant::now() + deadline, client.turn(&saved.actor)).await;
            let mut state = shared.lock();
            if state.pane_generation != generation {
                return;
            }
            state.check_task = None;
            let (turn, turn_state, selected) = match answer {
                Part::Ready(Some(turn)) => {
                    let actor = turn.actor.clone();
                    let (loaded, coming) = {
                        let list = shared.list.borrow();
                        (list.index(&actor).is_some(), list.more || list.paging != Paging::Idle)
                    };
                    if !loaded && coming {
                        state.pending = Some(actor.clone());
                    }
                    (Arc::new(turn), TurnState::Running, loaded.then_some(actor))
                }
                Part::Ready(None) => (saved, TurnState::Ended, None),
                Part::Failed(error) => (saved, TurnState::Failed(error), None),
            };
            shared.pane.send_modify(|pane| {
                pane.turn = Some(turn);
                pane.state = turn_state;
                pane.selected = selected;
            });
        }));
    }
}
