//! Live updates: one change stream (`GET /changes`, ADR 0035) for the page on screen, whose hints are gathered into
//! batches that run the page's soft refresh, one at a time. The server's `resync` of every new subscription is the
//! one recovery trigger. A page also refreshes 30 s after its last refresh attempt ended, as hints are best effort.
//! The stream is opened again with backoff after it ends or fails. The model runs as one task on the session's
//! runtime and publishes a [`LiveStatus`].

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::Duration;

use cicada::ActorAddress;
use cicada::time::Timestamp;
use cicada_client::{ActorClient, Change};
use futures::{Stream, StreamExt};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until};

use crate::model::{Section, Session, error_text};
use crate::refresh::Done;

/// The waits between tries to open the stream again: 1, 2, 4, 8, 16, then 30 s.
pub(crate) const BACKOFF: [Duration; 6] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(30),
];
/// How long a stream stays open after its first valid event before the backoff starts again from 1 s.
pub(crate) const STEADY: Duration = Duration::from_secs(60);
/// How long hints are gathered after the first of a batch.
pub(crate) const GATHER: Duration = Duration::from_millis(500);
/// The most addresses a batch names; past that it counts as everything, as a `resync` does.
pub(crate) const BATCH_LIMIT: usize = 1000;
/// How long after its last refresh attempt ended a page refreshes without hints.
pub(crate) const FALLBACK: Duration = Duration::from_secs(30);

/// An open subscription's hints, with errors as text.
pub(crate) type Hints = Pin<Box<dyn Stream<Item = Result<Change, String>> + Send>>;
type Opening = Pin<Box<dyn Future<Output = Result<Hints, String>> + Send>>;

/// Where change hints come from: [`ActorClient::changes`] in the app, a channel in tests.
pub(crate) trait ChangeSource: Send + Sync + 'static {
    /// Opens a subscription under the literal address `prefix`.
    fn open(&self, prefix: String) -> impl Future<Output = Result<Hints, String>> + Send + 'static;
}

impl ChangeSource for ActorClient {
    fn open(&self, prefix: String) -> impl Future<Output = Result<Hints, String>> + Send + 'static {
        let client = self.clone();
        async move {
            let stream = client.changes(&prefix).await.map_err(|error| error_text(&error))?;
            Ok(Box::pin(stream.map(|hint| hint.map_err(|error| error_text(&error)))) as Hints)
        }
    }
}

/// What the live model refreshes: the session's pages.
pub(crate) trait Pages: Send + Sync {
    /// Starts a soft refresh of `section`; [`Done`] resolves when the attempt ends, also when it is skipped.
    fn soft_refresh(&self, section: Section) -> Done;
    /// Cancels the soft refresh of `section` and drops its results.
    fn cancel_soft_refresh(&self, section: Section);
    /// The actor the pane of `section` shows.
    fn pane_actor(&self, section: Section) -> Option<ActorAddress>;
}

impl Pages for Session {
    fn soft_refresh(&self, section: Section) -> Done {
        Session::soft_refresh(self, section, None)
    }

    fn cancel_soft_refresh(&self, section: Section) {
        self.cancel_live_refresh(section);
    }

    fn pane_actor(&self, section: Section) -> Option<ActorAddress> {
        Session::pane_actor(self, section)
    }
}

/// What the Live pill shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LiveStatus {
    /// No context is in use, so there is nothing to follow.
    #[default]
    Off,
    /// The stream is opening, or waits for its first event.
    Connecting,
    /// The stream gave its first valid event.
    Live,
    /// The last stream ended or failed; the next try is at `at`.
    Reconnecting {
        /// When the next try starts.
        at: Timestamp,
        /// Why the last stream ended.
        reason: String,
    },
    /// The user paused the updates.
    Paused,
}

/// The prefix the stream of `section` subscribes to: the search prefix on the pages the server lists by prefix, else
/// every actor. A local filter, as on Types, Watches and Nodes, leaves the subscription alone.
pub(crate) fn subscription(section: Section, search: &str) -> &str {
    match section {
        Section::Actors | Section::Messages | Section::RunningTurns | Section::Outcomes => search,
        Section::Overview | Section::Nodes | Section::Types | Section::Watches => "",
    }
}

enum Command<S> {
    Source(Option<Arc<S>>),
    Show(Section, String),
}

/// The live model of one window. Dropping it closes the stream.
pub(crate) struct Live<S: ChangeSource = ActorClient> {
    commands: mpsc::UnboundedSender<Command<S>>,
    status: watch::Receiver<LiveStatus>,
    task: JoinHandle<()>,
}

impl<S: ChangeSource> Live<S> {
    /// A model of `pages` with no source yet, on `handle`. A new search prefix is followed once it has been still
    /// for `debounce`.
    pub fn new(handle: &Handle, pages: Weak<dyn Pages>, debounce: Duration) -> Self {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (status, watcher) = watch::channel(LiveStatus::Off);
        let driver = Driver {
            commands: receiver,
            status,
            pages,
            debounce,
            source: None,
            section: Section::Overview,
            prefix: String::new(),
            pending_prefix: None,
            link: Link::Closed,
            backoff: 0,
            batch: None,
            refreshing: None,
            timer: None,
        };
        Self { commands, status: watcher, task: handle.spawn(driver.run()) }
    }

    /// A context switch: the stream closes, and one opens on `source` when there is one.
    pub fn use_source(&self, source: Option<S>) {
        let _ = self.commands.send(Command::Source(source.map(Arc::new)));
    }

    /// The page on screen and its search text. A page switch reopens the stream at once; a new prefix of the same
    /// page after the debounce.
    pub fn show(&self, section: Section, search: &str) {
        let _ = self.commands.send(Command::Show(section, subscription(section, search).to_owned()));
    }

    /// The pill's states, newest last.
    pub fn status(&self) -> watch::Receiver<LiveStatus> {
        self.status.clone()
    }
}

impl<S: ChangeSource> Drop for Live<S> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

enum Link {
    Closed,
    Opening(Opening),
    Open { hints: Hints, first: Option<Instant> },
    Waiting(Instant),
}

enum Event<S> {
    Command(Option<Command<S>>),
    Opened(Result<Hints, String>),
    Hint(Option<Result<Change, String>>),
    Retry,
    Prefix,
    Batch,
    Ended,
    Timer,
}

/// The hints gathered for the next soft refresh.
struct Batch {
    /// When it may start: [`GATHER`] after its first hint.
    due: Instant,
    addresses: BTreeSet<ActorAddress>,
    /// A `resync`, or more than [`BATCH_LIMIT`] addresses.
    everything: bool,
}

struct Driver<S> {
    commands: mpsc::UnboundedReceiver<Command<S>>,
    status: watch::Sender<LiveStatus>,
    pages: Weak<dyn Pages>,
    debounce: Duration,
    source: Option<Arc<S>>,
    section: Section,
    /// The prefix subscribed to.
    prefix: String,
    /// A new prefix of the same page and when it is followed.
    pending_prefix: Option<(String, Instant)>,
    link: Link,
    /// The index into [`BACKOFF`] of the next wait.
    backoff: usize,
    batch: Option<Batch>,
    /// The soft refresh this model started, until its attempt ends.
    refreshing: Option<Done>,
    /// When the page refreshes without hints; none while a refresh runs.
    timer: Option<Instant>,
}

impl<S: ChangeSource> Driver<S> {
    async fn run(mut self) {
        loop {
            match self.next().await {
                Event::Command(None) => return,
                Event::Command(Some(command)) => self.command(command),
                Event::Opened(Ok(hints)) => self.link = Link::Open { hints, first: None },
                Event::Opened(Err(error)) => self.fail(error),
                Event::Hint(Some(Ok(change))) => self.hint(change),
                Event::Hint(Some(Err(error))) => self.fail(error),
                Event::Hint(None) => self.fail("the server ended the change stream".into()),
                Event::Retry => self.open(),
                Event::Prefix => {
                    if let Some((prefix, _)) = self.pending_prefix.take() {
                        self.prefix = prefix;
                        self.restart();
                    }
                }
                Event::Batch => {
                    if let Some(batch) = self.batch.take()
                        && self.wants(&batch)
                    {
                        self.start();
                    }
                }
                Event::Ended => {
                    self.refreshing = None;
                    self.arm();
                }
                Event::Timer => {
                    self.timer = None;
                    self.start();
                }
            }
        }
    }

    async fn next(&mut self) -> Event<S> {
        let Self { commands, link, pending_prefix, batch, refreshing, timer, .. } = self;
        let link = async {
            match link {
                Link::Closed => std::future::pending().await,
                Link::Opening(opening) => Event::Opened(opening.await),
                Link::Open { hints, .. } => Event::Hint(hints.next().await),
                Link::Waiting(at) => {
                    sleep_until(*at).await;
                    Event::Retry
                }
            }
        };
        let prefix = async {
            match pending_prefix {
                Some((_, at)) => sleep_until(*at).await,
                None => std::future::pending().await,
            }
        };
        // A batch and the timer wait while a refresh runs; its end lets them start.
        let idle = refreshing.is_none();
        let batch = async {
            match batch {
                Some(batch) if idle => sleep_until(batch.due).await,
                _ => std::future::pending().await,
            }
        };
        let timer = async {
            match timer {
                Some(at) if idle => sleep_until(*at).await,
                _ => std::future::pending().await,
            }
        };
        let ended = async {
            match refreshing {
                Some(done) => done.await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            command = commands.recv() => Event::Command(command),
            event = link => event,
            () = prefix => Event::Prefix,
            () = ended => Event::Ended,
            () = batch => Event::Batch,
            () = timer => Event::Timer,
        }
    }

    fn command(&mut self, command: Command<S>) {
        match command {
            Command::Source(source) => {
                self.source = source;
                self.pending_prefix = None;
                self.restart();
            }
            Command::Show(section, prefix) if section != self.section => {
                self.section = section;
                self.prefix = prefix;
                self.pending_prefix = None;
                self.restart();
            }
            Command::Show(_, prefix) if prefix == self.prefix => self.pending_prefix = None,
            Command::Show(_, prefix) => {
                if self.pending_prefix.as_ref().is_none_or(|(pending, _)| *pending != prefix) {
                    self.pending_prefix = Some((prefix, Instant::now() + self.debounce));
                }
            }
        }
    }

    fn hint(&mut self, change: Change) {
        if let Link::Open { first: first @ None, .. } = &mut self.link {
            *first = Some(Instant::now());
            self.status.send_replace(LiveStatus::Live);
        }
        let batch = || Batch { due: Instant::now() + GATHER, addresses: BTreeSet::new(), everything: false };
        match change {
            Change::Changed(address) => {
                let batch = self.batch.get_or_insert_with(batch);
                if !batch.everything {
                    batch.addresses.insert(address);
                    if batch.addresses.len() > BATCH_LIMIT {
                        batch.addresses.clear();
                        batch.everything = true;
                    }
                }
            }
            Change::Resync => {
                let batch = self.batch.get_or_insert_with(batch);
                batch.addresses.clear();
                batch.everything = true;
            }
            _ => {}
        }
    }

    /// Whether `batch` concerns the page: on the paged pages with an actor per row, an address under the prefix or
    /// the pane's actor; on the other pages any hint.
    fn wants(&self, batch: &Batch) -> bool {
        if batch.everything {
            return true;
        }
        match self.section {
            Section::Actors | Section::Messages | Section::RunningTurns | Section::Outcomes => {
                batch.addresses.iter().any(|address| address.as_str().starts_with(&self.prefix))
                    || self
                        .pages
                        .upgrade()
                        .and_then(|pages| pages.pane_actor(self.section))
                        .is_some_and(|actor| batch.addresses.contains(&actor))
            }
            Section::Overview | Section::Nodes | Section::Types | Section::Watches => true,
        }
    }

    /// Starts the page's soft refresh; the timer waits for its end.
    fn start(&mut self) {
        self.timer = None;
        if let Some(pages) = self.pages.upgrade() {
            self.refreshing = Some(pages.soft_refresh(self.section));
        }
    }

    /// Arms the timer [`FALLBACK`] from now while there is a source to follow.
    fn arm(&mut self) {
        self.timer = self.source.is_some().then(|| Instant::now() + FALLBACK);
    }

    /// Closes the stream and opens the next at once, the backoff from the start. The batch of the stream before is
    /// dropped: the next stream's `resync` covers it.
    fn restart(&mut self) {
        self.link = Link::Closed;
        self.backoff = 0;
        self.batch = None;
        if self.refreshing.is_none() {
            self.arm();
        }
        self.status.send_replace(if self.source.is_some() { LiveStatus::Connecting } else { LiveStatus::Off });
        self.open();
    }

    fn open(&mut self) {
        self.link = match &self.source {
            Some(source) => Link::Opening(Box::pin(source.open(self.prefix.clone()))),
            None => Link::Closed,
        };
    }

    /// The stream ended or failed: the next try waits for the backoff, which starts again after a steady stream.
    fn fail(&mut self, reason: String) {
        if let Link::Open { first: Some(first), .. } = &self.link
            && first.elapsed() >= STEADY
        {
            self.backoff = 0;
        }
        let wait = BACKOFF[self.backoff.min(BACKOFF.len() - 1)];
        self.backoff += 1;
        self.link = Link::Waiting(Instant::now() + wait);
        let at = chrono::Utc::now() + wait;
        self.status.send_replace(LiveStatus::Reconnecting { at, reason });
    }
}

#[cfg(test)]
pub(crate) mod channel {
    //! A change source for tests: every open is recorded, and the test feeds or ends each stream.

    use std::sync::{Arc, Mutex};

    use cicada_client::Change;
    use tokio::sync::mpsc;

    use super::{ChangeSource, Hints};

    /// One try to open a subscription: its prefix, when it came, and the sender of its hints, or why it failed.
    /// Dropping the sender ends the stream.
    pub struct Opened {
        /// The subscribed prefix.
        pub prefix: String,
        /// When the try came.
        pub at: tokio::time::Instant,
        /// Feeds the stream.
        pub hints: mpsc::UnboundedSender<Result<Change, String>>,
        /// The error the try answered with instead of a stream.
        pub failed: Option<String>,
    }

    /// Records each try, answering with a stream fed by the test, or with the next queued error.
    #[derive(Clone)]
    pub struct Channel {
        opened: mpsc::UnboundedSender<Opened>,
        failures: Arc<Mutex<Vec<String>>>,
    }

    impl Channel {
        /// A source and the receiver of its tries.
        pub fn new() -> (Self, mpsc::UnboundedReceiver<Opened>) {
            let (sender, receiver) = mpsc::unbounded_channel();
            (Self { opened: sender, failures: Arc::default() }, receiver)
        }

        /// The next opens fail with these errors, in order.
        pub fn fail_next(&self, errors: impl IntoIterator<Item = &'static str>) {
            self.failures.lock().unwrap().extend(errors.into_iter().map(str::to_owned));
        }
    }

    impl ChangeSource for Channel {
        fn open(&self, prefix: String) -> impl Future<Output = Result<Hints, String>> + Send + 'static {
            let failure = {
                let mut failures = self.failures.lock().unwrap();
                (!failures.is_empty()).then(|| failures.remove(0))
            };
            let (hints, mut receiver) = mpsc::unbounded_channel();
            let at = tokio::time::Instant::now();
            let _ = self.opened.send(Opened { prefix, at, hints, failed: failure.clone() });
            async move {
                match failure {
                    Some(error) => Err(error),
                    None => Ok(Box::pin(futures::stream::poll_fn(move |cx| receiver.poll_recv(cx))) as Hints),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
