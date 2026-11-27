//! The root pager of a paged page: the server's cursor passed back unchanged, at most one page in flight, Retry from
//! the cursor it kept, and a generation that grows on a new list, prefix, status, Refresh or context. A new search
//! waits 300 ms after the last key. The page keeps the pager under its state lock and cancels it on context switch
//! or close. Tasks find it through [`Paged`].

use std::collections::HashSet;
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, MutexGuard};

use cicada::api::PageView;
use cicada::time::Timestamp;
use cicada_client::{ActorClient, ClientError};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::model::{Limits, Part, within};
use crate::refresh::{Budget, Readiness, Refresh, RefreshPage, Stale, as_of};

/// The rows of one page.
pub const PAGE: usize = 100;
/// The view asks for the next page when the last visible row is this close to the end.
pub const NEAR_END: usize = 20;

/// The state of a list's paging.
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

/// Where the next page starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Cursor {
    /// The first page is not read yet.
    #[default]
    Start,
    /// The server's cursor of the next page, passed back unchanged.
    Next(String),
    /// The last page is read.
    End,
}

/// A page that answered under the generation it was asked in.
pub(crate) struct Landed<T> {
    /// The generation it was asked under, which is still the pager's.
    pub generation: u64,
    /// True for the first page of the generation.
    pub first: bool,
    /// Its rows and whether the server has more after them, or why it failed.
    pub page: Part<(Vec<T>, bool)>,
}

/// The owner of a pager: its shared part, whose lock guards the state that holds the pager.
pub(crate) trait Paged: Send + Sync + 'static {
    /// The state behind the lock.
    type State: Send;

    /// The runtime the reads run on.
    fn handle(&self) -> &Handle;

    /// Locks the state.
    fn lock(&self) -> MutexGuard<'_, Self::State>;

    /// The pager in the state.
    fn pager(state: &mut Self::State) -> &mut Pager;
}

/// The paging of a root list. Every page and every pending search is bound to the generation it was asked under.
#[derive(Default)]
pub(crate) struct Pager {
    generation: u64,
    cursor: Cursor,
    /// The cursor of the page out under the current generation.
    asked: Option<Option<String>>,
    task: Option<JoinHandle<()>>,
    debounce: Option<JoinHandle<()>>,
    /// Soft refreshes own a separate generation.
    pub refresh: Refresh,
    deferred_page: bool,
    read_at: Option<Timestamp>,
}

impl Pager {
    /// The current generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Drops the page out and a pending search under a new generation, whose next page is the first; returns their
    /// tasks for the owner to abort.
    pub fn restart(&mut self) -> [Option<JoinHandle<()>>; 2] {
        self.generation += 1;
        self.cursor = Cursor::Start;
        self.refresh.cancel();
        self.deferred_page = false;
        self.read_at = None;
        self.asked = None;
        [self.task.take(), self.debounce.take()]
    }

    /// When a page last answered.
    pub fn read_at(&self) -> Option<Timestamp> {
        self.read_at
    }

    /// Cancels the soft refresh and any work requested against its cursor.
    pub fn cancel_refresh(&mut self) {
        self.refresh.cancel();
        self.deferred_page = false;
    }

    /// True when a page can be asked for: the last is not read and none is out.
    pub fn can_ask(&self) -> bool {
        self.cursor != Cursor::End && self.asked.is_none()
    }

    /// Reads the page at the cursor within `limits.deadline`, unless [`Self::can_ask`] is false; `land` gets it with
    /// the state locked, only while its generation and cursor are still the pager's, after the cursor moved on. A
    /// failed page keeps the cursor, so asking again is Retry. Returns true when the read started.
    pub fn request<O, T, F, Fut, L>(owner: &Arc<O>, state: &mut O::State, limits: Limits, read: F, land: L) -> bool
    where
        O: Paged,
        T: Send + 'static,
        F: FnOnce(Option<String>) -> Fut,
        Fut: Future<Output = Result<PageView<T>, ClientError>> + Send + 'static,
        L: FnOnce(&Arc<O>, &mut O::State, Landed<T>) + Send + 'static,
    {
        let pager = O::pager(state);
        if pager.refresh.running() {
            pager.deferred_page = true;
            return false;
        }
        if !pager.can_ask() {
            return false;
        }
        let cursor = match &pager.cursor {
            Cursor::Next(cursor) => Some(cursor.clone()),
            Cursor::Start | Cursor::End => None,
        };
        let (generation, first) = (pager.generation, pager.cursor == Cursor::Start);
        pager.asked = Some(cursor.clone());
        let call = read(cursor.clone());
        let owner_task = owner.clone();
        let task = owner.handle().spawn(async move {
            let page = within(Instant::now() + limits.deadline, call).await;
            let owner = owner_task;
            let mut state = owner.lock();
            let pager = O::pager(&mut state);
            if pager.generation != generation || pager.asked.as_ref() != Some(&cursor) {
                return;
            }
            pager.asked = None;
            pager.task = None;
            let page = match page {
                Part::Ready(page) => {
                    pager.read_at = Some(chrono::Utc::now());
                    pager.cursor = page.next.map_or(Cursor::End, Cursor::Next);
                    Part::Ready((page.items, pager.cursor != Cursor::End))
                }
                Part::Failed(error) => Part::Failed(error),
            };
            land(&owner, &mut state, Landed { generation, first, page });
        });
        O::pager(state).task = Some(task);
        true
    }

    /// Runs `then` with the state locked once `limits.debounce` has passed, unless the generation moved on first.
    pub fn debounce<O, T>(owner: &Arc<O>, state: &mut O::State, limits: Limits, then: T)
    where
        O: Paged,
        T: FnOnce(&Arc<O>, &mut O::State) + Send + 'static,
    {
        let generation = O::pager(state).generation;
        let owner_task = owner.clone();
        let task = owner.handle().spawn(async move {
            tokio::time::sleep(limits.debounce).await;
            let owner = owner_task;
            let mut state = owner.lock();
            let pager = O::pager(&mut state);
            if pager.generation == generation {
                pager.debounce = None;
                then(&owner, &mut state);
            }
        });
        O::pager(state).debounce = Some(task);
    }
}

/// A paged list with a soft refresh. The range reader can also be used independently for an address-ordered tree.
pub(crate) trait SoftPaged: Paged {
    /// A server row.
    type Row: Send + Sync;
    /// The row's identity.
    type Key: Ord + Hash + Send + Sync;
    /// The server's order of the rows, as documented by the store, compared like the server does.
    type Sort: Ord + Send + Sync;
    /// The query, apart from the cursor.
    type Query: Clone + Send + Sync;

    /// The row's identity.
    fn key(row: &Self::Row) -> Self::Key;
    /// The row's place in the server's order.
    fn sort(row: &Self::Row) -> Self::Sort;
    /// The loaded rows, including those hidden by a local filter.
    fn rows(&self, state: &Self::State) -> Vec<Arc<Self::Row>>;
    /// The current paging status.
    fn paging(&self) -> Paging;
    /// The refresh deadline.
    fn limits(&self) -> Limits;
    /// The mark already on the list.
    fn stale(&self) -> Option<Stale>;
    /// The current query, or none without a client or an initial read.
    fn query(state: &Self::State) -> Option<(ActorClient, Self::Query)>;
    /// Reads one page using the server's cursor.
    fn read_page(
        client: ActorClient,
        query: Self::Query,
        cursor: Option<String>,
    ) -> impl Future<Output = Result<PageView<Self::Row>, ClientError>> + Send;
    /// Replaces the rows if any pages answered, and sets or clears the mark.
    fn land_refresh(
        self: &Arc<Self>,
        state: &mut Self::State,
        rows: Option<Vec<Arc<Self::Row>>>,
        more: bool,
        stale: Option<Stale>,
    );
    /// Refreshes a pane that needs a separate read, if the selection still calls for it.
    fn refresh_pane(self: &Arc<Self>, _ticket: u64, _budget: Budget) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
    /// Asks for the next ordinary page.
    fn page(self: &Arc<Self>, state: &mut Self::State);
}

impl<O: SoftPaged> RefreshPage for O {
    type State = O::State;

    fn lock_refresh(&self) -> MutexGuard<'_, Self::State> {
        self.lock()
    }

    fn refresh(state: &mut Self::State) -> &mut Refresh {
        &mut Self::pager(state).refresh
    }

    fn runtime(&self) -> (&Handle, std::time::Duration) {
        (self.handle(), self.limits().deadline)
    }

    fn readiness(&self, state: &Self::State) -> Readiness {
        match self.paging() {
            Paging::Loading => Readiness::Wait,
            Paging::Idle if Self::query(state).is_some() => Readiness::Ready,
            _ => Readiness::Skip,
        }
    }

    async fn run(self: &Arc<Self>, ticket: u64, budget: Budget) {
        let ((client, query), old) = {
            let mut state = self.lock();
            if !Self::refresh(&mut state).current(ticket) {
                return;
            }
            let Some(read) = Self::query(&state) else { return };
            (read, self.rows(&state))
        };
        let read = |cursor| Self::read_page(client.clone(), query.clone(), cursor);
        let old_last = old.last().map(|row| Self::sort(row));
        let range = read_range(&budget, read, old_last.as_ref(), Self::sort).await;
        {
            let mut state = self.lock();
            let pager = Self::pager(&mut state);
            if !pager.refresh.current(ticket) {
                return;
            }
            let old_time = as_of(self.stale().as_ref(), pager.read_at);
            let (rows, kept) = if let Some(page) = range.page {
                pager.cursor = page.next.map_or(Cursor::End, Cursor::Next);
                pager.read_at = Some(chrono::Utc::now());
                let (rows, kept) = replace_range(old, page.items, pager.cursor != Cursor::End, Self::key, Self::sort);
                (Some(rows), kept)
            } else {
                (None, false)
            };
            let stale = (kept || range.error.is_some()).then_some(Stale {
                as_of: old_time,
                capped: kept && range.error.is_none(),
                error: range.error,
            });
            let more = matches!(pager.cursor, Cursor::Next(_));
            self.land_refresh(&mut state, rows, more, stale);
        }
        self.refresh_pane(ticket, budget).await;
    }

    fn after(self: &Arc<Self>, state: &mut Self::State) {
        if std::mem::take(&mut Self::pager(state).deferred_page) {
            self.page(state);
        }
    }
}

/// What answered from the beginning of one paged list.
pub(crate) struct Range<T> {
    /// The accumulated rows and the last cursor received, if a page answered.
    pub page: Option<PageView<T>>,
    /// The error when a later page (or the first one) failed or missed the deadline.
    pub error: Option<String>,
}

/// Re-reads from the start until the old last row is covered, the cursor ends or three pages have answered.
pub(crate) async fn read_range<T, S, Fut>(
    budget: &Budget,
    read: impl Fn(Option<String>) -> Fut,
    old_last: Option<&S>,
    sort: impl Fn(&T) -> S,
) -> Range<T>
where
    S: Ord,
    Fut: Future<Output = Result<PageView<T>, ClientError>>,
{
    let mut range: Option<PageView<T>> = None;
    for _ in 0..3 {
        let cursor = range.as_ref().and_then(|page| page.next.clone());
        let page = match budget.read(read(cursor)).await {
            Part::Ready(page) => page,
            Part::Failed(error) => return Range { page: range, error: Some(error) },
        };
        let covered = old_last.is_none_or(|last| page.items.last().is_some_and(|row| sort(row) >= *last));
        let end = page.next.is_none();
        let all = range.get_or_insert_with(|| PageView { items: Vec::new(), next: None });
        all.items.extend(page.items);
        all.next = page.next;
        if covered || end {
            break;
        }
    }
    Range { page: range, error: None }
}

/// Replaces the covered range, keeping the old rows that sort after its last row and dropping duplicate identities.
pub(crate) fn replace_range<T, K: Ord + Hash, S: Ord>(
    old: Vec<Arc<T>>,
    fresh: Vec<T>,
    more: bool,
    key: impl Fn(&T) -> K,
    sort: impl Fn(&T) -> S,
) -> (Vec<Arc<T>>, bool) {
    let mut seen = HashSet::new();
    let boundary = fresh.last().map(&sort);
    let mut rows: Vec<_> = fresh.into_iter().filter(|row| seen.insert(key(row))).map(Arc::new).collect();
    let fresh_count = rows.len();
    if more {
        rows.extend(
            old.into_iter()
                .filter(|row| boundary.as_ref().is_none_or(|last| sort(row) > *last) && seen.insert(key(row))),
        );
    }
    let kept = rows.len() > fresh_count;
    (rows, kept)
}

/// Inserts an ordered page among the held rows and ignores keys the list already has.
pub(crate) fn insert_ordered<T, K: Ord>(rows: &mut Vec<Arc<T>>, fresh: Vec<T>, key: impl Fn(&T) -> K) -> usize {
    let before = rows.len();
    for row in fresh {
        let id = key(&row);
        if let Err(index) = rows.binary_search_by_key(&id, |row| key(row)) {
            rows.insert(index, Arc::new(row));
        }
    }
    rows.len() - before
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::replace_range;

    /// A pending message as `(actor, place in its mailbox, id)`, sorted like the server.
    type Row = (&'static str, u32, String);

    fn rows(actor: &'static str, ids: std::ops::Range<u32>) -> Vec<Row> {
        ids.map(|n| (actor, 1000 + n, format!("{actor}{n:03}"))).collect()
    }

    fn refresh(old: Vec<Row>, fresh: Vec<Row>) -> Vec<String> {
        let old = old.into_iter().map(Arc::new).collect();
        let (rows, _) =
            replace_range(old, fresh, true, |row: &Row| row.2.clone(), |row: &Row| (row.0, row.1, row.2.clone()));
        rows.iter().map(|row| row.2.clone()).collect()
    }

    #[test]
    fn a_row_moved_to_the_end_of_the_range_keeps_the_rows_after_it() {
        // Actor a has 299 rows and c 150; Deliver Now puts c140 first in c's block, the range's last place.
        let old = [rows("a", 0..299), rows("c", 0..150)].concat();
        let mut fresh = rows("a", 0..299);
        fresh.push(("c", 0, "c140".to_owned()));
        let ids = refresh(old, fresh);
        assert_eq!(ids.len(), 449, "every pending row once");
        assert_eq!(ids[299], "c140");
        assert_eq!(ids[300], "c000");
    }

    #[test]
    fn many_rows_moved_ahead_keep_the_rows_they_passed() {
        // Deliver Now on m280..m439 of 450: they move ahead of every other row.
        let old = rows("m", 0..450);
        let moved = (280..439).map(|n| ("m", n - 280, format!("m{n:03}")));
        let fresh: Vec<Row> = moved.chain(rows("m", 0..141)).take(300).collect();
        let ids = refresh(old, fresh);
        assert_eq!(ids.len(), 450, "every pending row once");
        assert!(ids.iter().any(|id| id == "m140"));
    }
}
