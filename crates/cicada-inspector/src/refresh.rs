//! Coordination and request limits shared by the inspector's soft refreshes.

use std::future::Future;
use std::sync::{Arc, MutexGuard};
use std::time::Duration;

use chrono::Utc;
use cicada::time::Timestamp;
use cicada_client::ClientError;
use tokio::{
    runtime::Handle,
    sync::Semaphore,
    task::JoinHandle,
    time::{Instant, timeout_at},
};

use crate::model::{Part, TIMED_OUT, error_text};

/// Rows carried over from an earlier read, with the reason when the new read did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stale {
    /// When the carried-over rows were read.
    pub as_of: Timestamp,
    /// True when the page limit stopped the read.
    pub capped: bool,
    /// Why a read failed or missed the deadline.
    pub error: Option<String>,
}

impl Stale {
    /// Marks an answer that could not be updated.
    pub fn failed(as_of: Timestamp, error: String) -> Self {
        Self { as_of, capped: false, error: Some(error) }
    }

    /// The note beside the row count.
    pub fn text(&self) -> String {
        let rows = if self.capped { "rows past 300" } else { "not refreshed" };
        format!("{rows} as of {}", self.as_of.with_timezone(&chrono::Local).format("%H:%M"))
    }
}

/// One deadline and four request slots for the entire refresh, including any pane reads.
#[derive(Clone)]
pub(crate) struct Budget {
    deadline: Instant,
    slots: Arc<Semaphore>,
    concise_errors: bool,
}

impl Budget {
    /// Starts the clock for one refresh.
    pub fn new(deadline: Duration) -> Self {
        Self { concise_errors: true, ..Self::with_slots(deadline.min(Duration::from_secs(10)), 4) }
    }

    /// A separate request limit for an ordinary Overview read.
    pub fn with_slots(deadline: Duration, slots: usize) -> Self {
        Self { deadline: Instant::now() + deadline, slots: Arc::new(Semaphore::new(slots)), concise_errors: false }
    }

    /// The absolute end of this read.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Waits for a slot and reads within the original deadline.
    pub async fn read<T>(&self, call: impl Future<Output = Result<T, ClientError>>) -> Part<T> {
        let result = timeout_at(self.deadline, async {
            let _slot = self.slots.acquire().await.expect("refresh slots stay open");
            if Instant::now() >= self.deadline {
                return Ok(None);
            }
            call.await.map(Some)
        })
        .await;
        match result {
            Ok(Ok(Some(value))) => Part::Ready(value),
            Ok(Err(error)) => Part::Failed(if self.concise_errors { refresh_error(error) } else { error_text(&error) }),
            Ok(Ok(None)) | Err(_) => Part::Failed(TIMED_OUT.into()),
        }
    }
}

// A response can echo a credential or a request URL in its problem text. Only keep the status or the error kind.
fn refresh_error(error: ClientError) -> String {
    match error {
        ClientError::Api(problem) => format!("HTTP {}", problem.status),
        ClientError::Http(_) => "HTTP request failed".into(),
        ClientError::Body(_) => "Unexpected response body".into(),
        ClientError::Invalid(_) => "Could not make the request".into(),
    }
}

/// Whether a soft refresh can start.
pub(crate) enum Readiness {
    /// The list is ready to re-read.
    Ready,
    /// An ordinary read or Retry must finish first.
    Wait,
    /// The page has no client, has not been read, or must be retried by the user.
    Skip,
}

/// The separate generation and the single remembered request of a soft refresh.
#[derive(Default)]
pub(crate) struct Refresh {
    generation: u64,
    pending: bool,
    task: Option<JoinHandle<()>>,
}

impl Refresh {
    /// An ordinary read must leave this refresh's cursor alone.
    pub fn running(&self) -> bool {
        self.task.is_some()
    }

    /// Whether a result is still wanted.
    pub fn current(&self, ticket: u64) -> bool {
        self.running() && self.generation == ticket
    }

    /// Cancels this refresh and the request waiting for it.
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.pending = false;
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// A page with its own lock and refresh slot.
pub(crate) trait RefreshPage: Send + Sync + 'static {
    /// State held under the page's lock.
    type State: Send;

    /// Locks the page.
    fn lock_refresh(&self) -> MutexGuard<'_, Self::State>;
    /// The page's slot, under that lock.
    fn refresh(state: &mut Self::State) -> &mut Refresh;
    /// The runtime and deadline for reads.
    fn runtime(&self) -> (&Handle, Duration);
    /// Whether the ordinary list read allows this refresh.
    fn readiness(&self, state: &Self::State) -> Readiness;
    /// Takes the request's inputs under the lock before a later request can replace them.
    fn starting(&self, _state: &mut Self::State) {}

    /// Re-reads the page, checking the ticket under the lock before applying anything.
    fn run(self: &Arc<Self>, ticket: u64, budget: Budget) -> impl Future<Output = ()> + Send;

    /// Lets ordinary paging continue after giving the refresh its slot back.
    fn after(self: &Arc<Self>, _state: &mut Self::State) {}

    /// Requests a refresh, remembering it once if a read is already in progress.
    fn request(self: &Arc<Self>) {
        let mut state = self.lock_refresh();
        self.request_locked(&mut state);
    }

    /// Requests a refresh with the state already locked, alongside any inputs that belong to this request.
    fn request_locked(self: &Arc<Self>, state: &mut Self::State) {
        Self::refresh(state).pending = true;
        self.resume(state);
    }

    /// Gives a remembered request a chance to start after a read finishes.
    fn resume(self: &Arc<Self>, state: &mut Self::State) {
        let refresh = Self::refresh(state);
        if !refresh.pending || refresh.running() {
            return;
        }
        let readiness = self.readiness(state);
        let refresh = Self::refresh(state);
        match readiness {
            Readiness::Skip => refresh.pending = false,
            Readiness::Wait => {}
            Readiness::Ready => {
                refresh.pending = false;
                refresh.generation += 1;
                let ticket = refresh.generation;
                self.starting(state);
                let (handle, deadline) = self.runtime();
                let budget = Budget::new(deadline);
                let page = self.clone();
                Self::refresh(state).task = Some(handle.spawn(async move {
                    page.run(ticket, budget).await;
                    let mut state = page.lock_refresh();
                    if Self::refresh(&mut state).current(ticket) {
                        Self::refresh(&mut state).task = None;
                        page.after(&mut state);
                        page.resume(&mut state);
                    }
                }));
            }
        }
    }
}

/// The time of the first carried-over read, or a fallback when none has been published yet.
pub(crate) fn as_of(stale: Option<&Stale>, read_at: Option<Timestamp>) -> Timestamp {
    stale.map(|mark| mark.as_of).or(read_at).unwrap_or_else(Utc::now)
}
