//! The root pager of a paged page: the server's cursor passed back unchanged, at most one page in flight, Retry from
//! the cursor it kept, a generation that grows whenever the rows are replaced, the read of a new search 300 ms after
//! the last key, and cancellation on a context switch or close. The page that owns it keeps it in the state behind its
//! own lock, and the pager's tasks find it there again through [`Paged`].

use std::future::Future;
use std::sync::{Arc, MutexGuard};

use cicada::api::PageView;
use cicada_client::ClientError;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::model::{Limits, Part, within};

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
        self.asked = None;
        [self.task.take(), self.debounce.take()]
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
