//! `serve` and `worker`: open the store, build and start the node, run the host's work after the start, serve, and
//! stop in good order.

use std::future::Future;
use std::io::Write;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cicada::observer::TurnObserver;
use cicada::runtime::{ActorSystemBuilder, SourceError, TurnSource, Worker, WorkerBuilder, WorkerConfig};
use cicada::store::StoreError;
use cicada::{Actor, ActorSystem, ActorType, Store};
use cicada_axum::{actor_api, actor_health};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::settings::{HttpSettings, Settings, StoreSpec};
use crate::{AppError, WorkerReady};

/// How long `serve` and `worker` wait for a PostgreSQL store that answers "unavailable", and how often they try.
#[cfg_attr(not(feature = "postgres"), allow(dead_code))]
const STORE_WAIT: Duration = Duration::from_secs(60);
#[cfg_attr(not(feature = "postgres"), allow(dead_code))]
const STORE_RETRY: Duration = Duration::from_secs(1);
/// How long requests that are still open may take to end once the node stopped.
const DRAIN: Duration = Duration::from_secs(5);

/// How long a worker over HTTP waits for a server whose type listing answers "unavailable" before it starts, and how
/// often it asks.
const SERVER_WAIT: Duration = Duration::from_secs(60);
const SERVER_RETRY: Duration = Duration::from_secs(1);

/// A type of the binary: its name, and how a node or a worker hosts it.
pub(crate) struct HostedType {
    pub name: &'static str,
    pub host: Box<dyn FnOnce(Hosting) -> Hosting + Send>,
}

/// What hosts the types of a command: the node of `serve` and of a worker on the store, or the `Worker` of a worker
/// over HTTP. Each type moves once into the one that runs.
pub(crate) enum Hosting {
    /// A node.
    System(ActorSystemBuilder),
    /// A worker over a turn source.
    Worker(WorkerBuilder),
}

impl Hosting {
    /// Hosts `actor`, as both builders do.
    pub(crate) fn host<A: Actor>(self, actor: ActorType<A>) -> Self {
        match self {
            Self::System(builder) => Self::System(builder.host(actor)),
            Self::Worker(builder) => Self::Worker(builder.host(actor)),
        }
    }

    /// Adds `observer`, as both builders do.
    fn observe(self, observer: Arc<dyn TurnObserver>) -> Self {
        match self {
            Self::System(builder) => Self::System(builder.observe(observer)),
            Self::Worker(builder) => Self::Worker(builder.observe(observer)),
        }
    }

    /// `builder` with the types of `types` that `selected` names and every observer.
    fn with(
        mut self,
        types: Vec<HostedType>,
        selected: &[&'static str],
        observers: Vec<Arc<dyn TurnObserver>>,
    ) -> Self {
        for hosted in types.into_iter().filter(|t| selected.contains(&t.name)) {
            self = (hosted.host)(self);
        }
        observers.into_iter().fold(self, Self::observe)
    }
}

/// A function of `App::http`.
pub(crate) type HttpFn = Box<dyn FnOnce(Router, &ActorSystem) -> Router + Send>;
/// A function of `App::on_start`.
pub(crate) type OnStartFn = Box<dyn FnOnce(ActorSystem) -> BoxFuture<'static, anyhow::Result<()>> + Send>;
/// A function of `App::on_worker_ready`.
pub(crate) type OnWorkerReadyFn = Box<dyn FnOnce(WorkerReady) + Send>;

/// A task of `App::task`: its name and its function.
pub(crate) struct HostTask {
    pub name: String,
    pub run: Box<dyn FnOnce(ActorSystem, Stopping) -> BoxFuture<'static, anyhow::Result<()>> + Send>,
}

/// What the host gave `App` for a node.
pub(crate) struct Parts {
    pub types: Vec<HostedType>,
    pub observers: Vec<Arc<dyn TurnObserver>>,
    pub http: Vec<HttpFn>,
    pub on_start: Vec<OnStartFn>,
    pub on_worker_ready: Vec<OnWorkerReadyFn>,
    pub tasks: Vec<HostTask>,
}

/// The stop of a node: it is set once the future given to `run_with` completes, which is watched from the moment the
/// command begins.
pub(crate) struct Stop {
    token: CancellationToken,
    watcher: JoinHandle<()>,
}

impl Stop {
    /// Watches `shutdown` on a task of its own.
    pub(crate) fn watch(shutdown: impl Future<Output = ()> + Send + 'static) -> Self {
        let token = CancellationToken::new();
        let set = token.clone();
        let watcher = tokio::spawn(async move {
            shutdown.await;
            set.cancel();
        });
        Self { token, watcher }
    }

    fn is_set(&self) -> bool {
        self.token.is_cancelled()
    }

    async fn wait(&self) {
        self.token.cancelled().await
    }
}

impl Drop for Stop {
    fn drop(&mut self) {
        self.watcher.abort();
    }
}

/// The stop of a node as a task of [`App::task`](crate::App::task) sees it. It is set when the node's stop begins: on
/// SIGTERM or SIGINT (the future given to [`App::run_with`](crate::App::run_with) in a test), when a task ends before
/// the stop, or when `serve` fails after the tasks started. It is never unset.
#[derive(Debug, Clone)]
pub struct Stopping(CancellationToken);

impl Stopping {
    /// Completes once the stop has begun; at once when it has.
    pub async fn wait(&self) {
        self.0.cancelled().await
    }

    /// Whether the stop has begun.
    pub fn is_set(&self) -> bool {
        self.0.is_cancelled()
    }

    fn set(&self) {
        self.0.cancel()
    }
}

/// What the tasks of `App::task` record of their ends, shared with [`Tasks`].
#[derive(Default)]
struct TaskBook {
    /// The names of the tasks that have not recorded their end.
    running: Vec<String>,
    /// The error of the first task that ended before the stop.
    early_end: Option<anyhow::Error>,
}

/// The running tasks of `App::task` and their [`Stopping`], which the node's stop shares.
pub(crate) struct Tasks {
    stopping: Stopping,
    running: JoinSet<()>,
    book: Arc<std::sync::Mutex<TaskBook>>,
}

impl Tasks {
    /// Starts `tasks` on `system` in order and returns once each was polled once, so the code of each before its first
    /// `await` has run. Their `Stopping` is set by `stop` too.
    async fn start(tasks: Vec<HostTask>, system: &ActorSystem, stop: &Stop) -> Self {
        let stopping = Stopping(stop.token.child_token());
        let book = Arc::new(std::sync::Mutex::new(TaskBook::default()));
        let mut running = JoinSet::new();
        let mut began = Vec::with_capacity(tasks.len());
        for task in tasks {
            let (polled, first_poll) = tokio::sync::oneshot::channel();
            let (system, given) = (system.clone(), stopping.clone());
            let run = task.run;
            book.lock().expect("the book of the tasks").running.push(task.name.clone());
            let ending = TaskEnding { name: task.name, stopping: stopping.clone(), book: book.clone() };
            running.spawn(run_task(async move { run(system, given).await }, ending, polled));
            began.push(first_poll);
        }
        for first_poll in began {
            // An error: the task panicked in its first poll, which it recorded.
            let _ = first_poll.await;
        }
        Self { stopping, running, book }
    }

    /// Waits for the stop, then for the tasks within `grace` from it: a task that has not recorded its end by then is
    /// aborted with a warning and not waited for. The error of the first task that ended before the stop, if one did.
    /// At once without tasks.
    async fn finish(mut self, grace: Duration) -> Result<(), AppError> {
        if self.running.is_empty() {
            return Ok(());
        }
        self.stopping.wait().await;
        let deadline = tokio::time::Instant::now() + grace;
        let all_ended = async { while self.running.join_next().await.is_some() {} };
        let timed_out = tokio::time::timeout_at(deadline, all_ended).await.is_err();
        let mut book = self.book.lock().expect("the book of the tasks");
        if timed_out {
            for name in &book.running {
                tracing::warn!("Task {name} did not end within the shutdown grace period of {grace:?}; aborted");
            }
            // Dropping the set aborts the tasks without waiting: a task that never yields cannot be aborted.
        }
        book.early_end.take().map_or(Ok(()), |e| Err(AppError::Failed(e)))
    }
}

/// What a task needs to record its end.
struct TaskEnding {
    name: String,
    stopping: Stopping,
    book: Arc<std::sync::Mutex<TaskBook>>,
}

/// Runs `task`, sends `polled` after its first poll, and catches its panic. Then it records its end in the book before
/// it sets `stopping`, so that [`Tasks::finish`], which wakes on `stopping`, finds it: the first end before the stop is
/// the error of the command, a later one before the stop or an error or panic after it a warning.
async fn run_task(
    task: impl Future<Output = anyhow::Result<()>> + Send,
    ending: TaskEnding,
    polled: tokio::sync::oneshot::Sender<()>,
) {
    let mut task = std::pin::pin!(task);
    let mut polled = Some(polled);
    let first_poll = std::future::poll_fn(move |cx| {
        let poll = task.as_mut().poll(cx);
        if let Some(polled) = polled.take() {
            let _ = polled.send(());
        }
        poll
    });
    let end = AssertUnwindSafe(first_poll).catch_unwind().await.map_err(|panic| panic_text(panic.as_ref()));
    let TaskEnding { name, stopping, book } = ending;
    let early = {
        let mut book = book.lock().expect("the book of the tasks");
        book.running.retain(|running| *running != name);
        let early = !stopping.is_set();
        match (end, early) {
            (Ok(Ok(())), false) => {}
            (end, true) if book.early_end.is_none() => {
                let ended = format!("task {name} ended before the stop");
                book.early_end = Some(match end {
                    Ok(Ok(())) => anyhow!(ended),
                    Ok(Err(e)) => e.context(ended),
                    Err(panic) => anyhow!("{ended}: {panic}"),
                });
            }
            (end, early) => {
                let text = match end {
                    Ok(Ok(())) => "it returned".to_owned(),
                    Ok(Err(e)) => format!("{e:#}"),
                    Err(panic) => panic,
                };
                let when = if early { "also ended before the stop" } else { "failed after the stop" };
                tracing::warn!("Task {name} {when}: {text}");
            }
        }
        early
    };
    if early {
        stopping.set();
    }
}

/// Sets `Stopping` and stops the node and the tasks together, then closes the store: the error of a task that ended
/// before the stop, if one did.
async fn stop_all(system: &ActorSystem, store: &Store, tasks: Tasks, grace: Duration) -> Result<(), AppError> {
    tasks.stopping.set();
    let ((), ended) = tokio::join!(system.stop(), tasks.finish(grace));
    store.close().await;
    ended
}

/// `serve`: steps 1 to 6 of the start, then requests until the stop.
pub(crate) async fn serve(
    parts: Parts,
    settings: Settings,
    out: &mut (dyn Write + Send),
    stop: Stop,
) -> Result<(), AppError> {
    let Some(store) = open_unless_stopped(&settings, &stop).await? else { return Ok(()) };
    let listener = match TcpListener::bind(settings.listen).await {
        Ok(listener) => listener,
        Err(e) => {
            store.close().await;
            return Err(AppError::Failed(anyhow!(e).context(format!("cannot listen on {}", settings.listen))));
        }
    };
    let system = build(parts.types, parts.observers, &settings, &store);
    // A route of the host that clashes with one of the API makes axum panic here, before the node started.
    let routes = match std::panic::catch_unwind(AssertUnwindSafe(|| routes(parts.http, &system))) {
        Ok(routes) => routes,
        Err(panic) => {
            store.close().await;
            return Err(AppError::Failed(anyhow!("the HTTP routes cannot be built: {}", panic_text(panic.as_ref()))));
        }
    };
    if !start(&system, &store, parts.on_start, &stop).await? {
        return Ok(());
    }
    let tasks = Tasks::start(parts.tasks, &system, &stop).await;
    let grace = settings.node.shutdown_grace_period;
    let address = match listener.local_addr() {
        Ok(address) => address,
        Err(e) => {
            stop_all(&system, &store, tasks, grace).await?;
            return Err(AppError::Failed(e.into()));
        }
    };
    let written = writeln!(out, "Listening on http://{address}").and_then(|()| out.flush());
    if let Err(e) = written {
        stop_all(&system, &store, tasks, grace).await?;
        return Err(AppError::Failed(anyhow!(e).context("cannot write to the output")));
    }
    tracing::info!("Serving the HTTP API on {address}");
    serve_and_close(listener, routes, &system, &store, tasks, grace, DRAIN).await
}

/// The end of `serve`: requests until the stop (see [`serve_until_stopped`]), while the tasks are waited for (see
/// [`Tasks::finish`]); then the node stops (at once when the stop stopped it; else the server failed first and sets
/// `Stopping`), and the store is closed after the node and the tasks. An early end of a task wins over a failed server.
async fn serve_and_close(
    listener: TcpListener,
    routes: Router,
    system: &ActorSystem,
    store: &Store,
    tasks: Tasks,
    grace: Duration,
    drain: Duration,
) -> Result<(), AppError> {
    let stopping = tasks.stopping.clone();
    let tasks_done = CancellationToken::new();
    let serving = async {
        let served = serve_until_stopped(listener, routes, system, &stopping, tasks_done.clone(), drain).await;
        stopping.set();
        system.stop().await;
        served
    };
    let finishing = async {
        let ended = tasks.finish(grace).await;
        tasks_done.cancel();
        ended
    };
    let (served, ended) = tokio::join!(serving, finishing);
    store.close().await;
    ended?;
    served.context("the HTTP server failed").map_err(AppError::Failed)
}

/// `worker`: steps 1, 4 and 5 of the start, then work until the stop.
pub(crate) async fn worker(parts: Parts, settings: Settings, stop: Stop) -> Result<(), AppError> {
    let Some(store) = open_unless_stopped(&settings, &stop).await? else { return Ok(()) };
    let system = build(parts.types, parts.observers, &settings, &store);
    if !start(&system, &store, parts.on_start, &stop).await? {
        return Ok(());
    }
    let tasks = Tasks::start(parts.tasks, &system, &stop).await;
    tasks.stopping.wait().await;
    stop_all(&system, &store, tasks, settings.node.shutdown_grace_period).await
}

/// A worker over HTTP: waits for the server ([`wait_for_server`]), starts a `Worker` of the selected types on the
/// source, and runs until the stop or until the worker stops by itself. It opens no store and no listener, and never
/// stops or closes the source.
pub(crate) async fn http_worker(
    parts: Parts,
    settings: HttpSettings,
    source: Arc<dyn TurnSource>,
    stop: Stop,
) -> Result<(), AppError> {
    match wait_for_server(source.as_ref(), &stop).await {
        Ok(true) => {}
        Ok(false) => return Ok(()),
        Err(e) => return Err(AppError::Failed(anyhow!(e).context("the server is not ready"))),
    }
    let worker = build_worker(parts.types, parts.observers, &settings.types, settings.worker, source);
    let started = tokio::select! {
        biased;
        () = stop.wait() => None,
        started = worker.start() => Some(started),
    };
    match started {
        // The stop came while the contracts were registered: those already registered stay.
        None => {
            worker.stop().await;
            return stopped(&worker).await;
        }
        Some(Err(e)) => {
            worker.stop().await;
            return Err(AppError::Failed(anyhow!(e).context("the worker cannot start")));
        }
        Some(Ok(())) => {}
    }
    let ready = WorkerReady {
        grace_period: worker.config().shutdown_grace_period,
        cancel_timeout: worker.config().shutdown_cancel_timeout,
    };
    for f in parts.on_worker_ready {
        if let Err(panic) = std::panic::catch_unwind(AssertUnwindSafe(|| f(ready))) {
            worker.stop().await;
            return Err(AppError::Failed(anyhow!("App::on_worker_ready panicked: {}", panic_text(panic.as_ref()))));
        }
    }
    run_until_stopped(&worker, &stop).await
}

/// Runs a started `worker` until the stop, which stops it with its grace period and cancel timeout, or until it stops
/// by itself; then its end (see [`stopped`]).
async fn run_until_stopped(worker: &Worker, stop: &Stop) -> Result<(), AppError> {
    tokio::select! {
        biased;
        () = stop.wait() => worker.stop().await,
        _ = worker.stopped() => {}
    }
    stopped(worker).await
}

/// The end of a stopped `worker`: an error it recorded when it stopped by itself, also when a stop came at the same
/// time, else success.
async fn stopped(worker: &Worker) -> Result<(), AppError> {
    match worker.stopped().await {
        None => Ok(()),
        Some(e) => Err(AppError::Failed(anyhow!(e).context("the worker stopped"))),
    }
}

/// The `Worker` of `source` with the types `selected` picks, the host's observers and `config`.
fn build_worker(
    types: Vec<HostedType>,
    observers: Vec<Arc<dyn TurnObserver>>,
    selected: &[&'static str],
    config: WorkerConfig,
    source: Arc<dyn TurnSource>,
) -> Worker {
    match Hosting::Worker(Worker::builder(source).config(config)).with(types, selected, observers) {
        Hosting::Worker(builder) => builder.build(),
        Hosting::System(_) => unreachable!("a worker builder stays a worker builder"),
    }
}

/// Lists the types of `source` until it answers, while it answers "unavailable": [`SERVER_RETRY`] after each try, and
/// no try at or after [`SERVER_WAIT`] from the first one. A try that began before then may end later, within the timeout
/// of the client. False when the stop came first, which interrupts a try and a wait; an error other than "unavailable"
/// at once. A server that answers is no promise that the registration that follows succeeds. On tokio's clock, so a
/// test runs it on virtual time.
async fn wait_for_server(source: &dyn TurnSource, stop: &Stop) -> Result<bool, SourceError> {
    let deadline = tokio::time::Instant::now() + SERVER_WAIT;
    loop {
        let listed = tokio::select! {
            biased;
            () = stop.wait() => return Ok(false),
            listed = source.list_types() => listed,
        };
        match listed {
            Ok(_) => return Ok(true),
            Err(e @ SourceError::Unavailable(_)) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(e);
                }
                tracing::warn!("The server is unavailable; trying again in {SERVER_RETRY:?}: {e}");
                tokio::select! {
                    biased;
                    () = stop.wait() => return Ok(false),
                    () = tokio::time::sleep(SERVER_RETRY) => {}
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(e);
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// The store of `settings`, or `None` when the stop came while it was waited for.
async fn open_unless_stopped(settings: &Settings, stop: &Stop) -> Result<Option<Store>, AppError> {
    tokio::select! {
        biased;
        () = stop.wait() => Ok(None),
        store = open(settings) => store.map(Some).map_err(AppError::Failed),
    }
}

/// Opens the store already checked by `App`. Waits for PostgreSQL while it answers "unavailable".
async fn open(settings: &Settings) -> anyhow::Result<Store> {
    match &settings.store {
        StoreSpec::Sqlite(path) => {
            #[cfg(feature = "sqlite")]
            {
                let mut options = cicada::store::SqliteOptions::default();
                if let Some(retention) = settings.retention {
                    options.retention = retention;
                }
                Store::sqlite(path, options)
                    .await
                    .with_context(|| format!("cannot open the SQLite store {}", path.display()))
            }
            #[cfg(not(feature = "sqlite"))]
            {
                let _ = path;
                unreachable!("App checks the SQLite feature before opening the store")
            }
        }
        StoreSpec::Postgres(url) => {
            #[cfg(feature = "postgres")]
            {
                let options = postgres_options(settings);
                wait_for_store(|| Store::postgres(url, options.clone()))
                    .await
                    .context("cannot open the PostgreSQL store")
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = url;
                unreachable!("App checks the PostgreSQL feature before opening the store")
            }
        }
    }
}

/// The options of a PostgreSQL store from `settings`.
#[cfg(feature = "postgres")]
fn postgres_options(settings: &Settings) -> cicada::store::PostgresOptions {
    let mut options = cicada::store::PostgresOptions::for_schema(&settings.schema);
    options.max_pool_size = settings.pool_size(options.max_pool_size);
    if let Some(retention) = settings.retention {
        options.retention = retention;
    }
    options
}

/// Calls `open` until it gives a store or an error other than "unavailable", [`STORE_RETRY`] after each try, and starts
/// no try after [`STORE_WAIT`]: containers start in any order. A try itself can take seconds (about 10 s for an address
/// that does not answer), so the whole wait can be longer than [`STORE_WAIT`]; the store counts a wrong password as
/// unavailable, so it is tried again too. On tokio's clock, so a test runs it on virtual time.
#[cfg_attr(not(feature = "postgres"), allow(dead_code))]
async fn wait_for_store<F, Fut>(mut open: F) -> Result<Store, StoreError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Store, StoreError>>,
{
    let deadline = tokio::time::Instant::now() + STORE_WAIT;
    loop {
        match open().await {
            Err(e) if e.is_unavailable() && tokio::time::Instant::now() < deadline => {
                tracing::warn!("The store is unavailable; trying again in {STORE_RETRY:?}: {e}");
                tokio::time::sleep(STORE_RETRY).await;
            }
            opened => return opened,
        }
    }
}

/// The node of `settings` on `store`, with the types it picks and the host's observers.
fn build(
    types: Vec<HostedType>,
    observers: Vec<Arc<dyn TurnObserver>>,
    settings: &Settings,
    store: &Store,
) -> ActorSystem {
    let builder = ActorSystem::builder(store.clone()).config(settings.node.clone());
    match Hosting::System(builder).with(types, &settings.types, observers) {
        Hosting::System(builder) => builder.build(),
        Hosting::Worker(_) => unreachable!("a node builder stays a node builder"),
    }
}

/// The API with the host's functions applied in order, behind `/ready`, which `App` answers outside the host's router:
/// no layer of the host covers it, and a route `/ready` of the host is never reached.
fn routes(http: Vec<HttpFn>, system: &ActorSystem) -> Router {
    let api = http.into_iter().fold(actor_api(system.clone()), |router, f| f(router, system));
    actor_health(system.clone(), "/ready").fallback_service(api)
}

/// Starts `system` and runs the host's functions of `on_start`. False when the stop came first: the node is then stopped
/// and the store closed. An error of the start or of a function stops the node and closes the store too.
async fn start(system: &ActorSystem, store: &Store, on_start: Vec<OnStartFn>, stop: &Stop) -> Result<bool, AppError> {
    // A stop while the node starts is taken when the start has ended.
    if let Err(e) = system.start().await {
        store.close().await;
        return Err(AppError::Failed(anyhow!(e).context("the node cannot start")));
    }
    for f in on_start {
        let ran = tokio::select! {
            biased;
            () = stop.wait() => None,
            ran = f(system.clone()) => Some(ran),
        };
        match ran {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                system.stop().await;
                store.close().await;
                return Err(AppError::Failed(e));
            }
            None => break,
        }
    }
    if stop.is_set() {
        system.stop().await;
        store.close().await;
        return Ok(false);
    }
    Ok(true)
}

/// Serves `routes` on `listener` until `stopping` is set: the node and the tasks stop first (the node claims no more,
/// running turns get its grace period, and the API keeps answering while `/ready` answers `503`) until `tasks_done` is
/// set too, then the server stops, and requests that are still open get `drain` to end before they are cut off.
async fn serve_until_stopped(
    listener: TcpListener,
    routes: Router,
    system: &ActorSystem,
    stopping: &Stopping,
    tasks_done: CancellationToken,
    drain: Duration,
) -> std::io::Result<()> {
    let (stopped, node_stopped) = tokio::sync::oneshot::channel::<()>();
    let token = stopping.0.clone();
    let node = system.clone();
    let graceful = async move {
        token.cancelled().await;
        tokio::join!(node.stop(), tasks_done.cancelled());
        let _ = stopped.send(());
    };
    let cut_off = CancellationToken::new();
    let routes = routes.layer(axum::middleware::from_fn_with_state(cut_off.clone(), cut_off_after));
    // axum serves each connection on a task of its own, which the end of this future does not end: `cut_off` does.
    let server = std::future::IntoFuture::into_future(axum::serve(listener, routes).with_graceful_shutdown(graceful));
    let drained = async move {
        if node_stopped.await.is_err() {
            std::future::pending::<()>().await;
        }
        tokio::time::sleep(drain).await;
    };
    tokio::select! {
        served = server => served,
        () = drained => {
            tracing::info!("Requests still open {drain:?} after the node stopped are cut off");
            cut_off.cancel();
            Ok(())
        }
    }
}

/// Lets a request run until `cut_off` is set. Then a handler that has not answered is dropped and the request gets
/// `503`, and a body that is still being sent ends with an error, which closes its connection.
async fn cut_off_after(
    axum::extract::State(cut_off): axum::extract::State<CancellationToken>,
    request: Request,
    next: Next,
) -> Response {
    tokio::select! {
        response = next.run(request) => {
            let cut = Box::pin(cut_off.cancelled_owned());
            response.map(|body| Body::new(CutBody { body, cut }))
        }
        () = cut_off.cancelled() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// A body that ends with an error once its cut is set.
struct CutBody {
    body: Body,
    cut: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl http_body::Body for CutBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, axum::Error>>> {
        if self.cut.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Some(Err(axum::Error::new("cut off: the node stopped"))));
        }
        Pin::new(&mut self.body).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}

/// The message of a panic.
fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use axum::routing::get;

    use super::*;

    /// A store that answers "unavailable" until `up` has passed on tokio's clock.
    fn late_store(up: Duration) -> impl FnMut() -> std::future::Ready<Result<Store, StoreError>> {
        let started = tokio::time::Instant::now();
        move || {
            std::future::ready(if started.elapsed() < up {
                Err(StoreError::unavailable("connection refused"))
            } else {
                Ok(Store::memory())
            })
        }
    }

    /// Scenario: The database comes up late.
    #[tokio::test(start_paused = true)]
    async fn a_store_that_comes_up_late_is_waited_for() {
        let started = tokio::time::Instant::now();
        wait_for_store(late_store(Duration::from_secs(5))).await.expect("the store opens");
        assert!(started.elapsed() >= Duration::from_secs(5) && started.elapsed() < Duration::from_secs(7));
    }

    /// Scenario: The database stays away.
    #[tokio::test(start_paused = true)]
    async fn a_store_that_stays_away_fails_after_a_minute() {
        let started = tokio::time::Instant::now();
        let error = wait_for_store(late_store(Duration::from_secs(3600))).await.expect_err("it stays away");
        assert!(error.is_unavailable() && error.to_string().contains("unavailable"), "{error}");
        assert!(started.elapsed() >= STORE_WAIT && started.elapsed() <= STORE_WAIT + STORE_RETRY);
    }

    #[tokio::test(start_paused = true)]
    async fn an_error_other_than_unavailable_is_not_waited_for() {
        let started = tokio::time::Instant::now();
        let error = wait_for_store(|| std::future::ready(Err(StoreError::Invalid("a foreign database".into())))).await;
        assert!(matches!(error, Err(StoreError::Invalid(_))));
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    /// Scenario: A request that stays open, with a short wait in place of five seconds.
    #[tokio::test]
    async fn a_request_that_stays_open_does_not_hold_up_the_stop() {
        let system = ActorSystem::builder(Store::memory()).build();
        system.start().await.unwrap();
        let stream = || async {
            let first = futures::stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(b"first")) });
            Body::from_stream(futures::StreamExt::chain(first, futures::stream::pending()))
        };
        let routes =
            Router::new().route("/hang", get(std::future::pending::<&'static str>)).route("/stream", get(stream));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let stop = Stop::watch(async move {
            let _ = rx.await;
        });
        let serving = {
            let system = system.clone();
            tokio::spawn(async move {
                let (stopping, done) = (Stopping(stop.token.clone()), CancellationToken::new());
                done.cancel();
                serve_until_stopped(listener, routes, &system, &stopping, done, Duration::from_millis(300)).await
            })
        };
        let open = tokio::spawn(reqwest::get(format!("http://{address}/hang")));
        let mut streaming = reqwest::get(format!("http://{address}/stream")).await.unwrap();
        assert_eq!(streaming.chunk().await.unwrap().as_deref(), Some(&b"first"[..]));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = Instant::now();
        tx.send(()).unwrap();
        serving.await.unwrap().expect("served");
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(300) && took < Duration::from_secs(2), "{took:?}");
        assert_eq!(system.state(), cicada::runtime::NodeState::Stopped, "the node stopped first");
        let open = tokio::time::timeout(Duration::from_secs(2), open).await.expect("cut off").unwrap();
        assert_eq!(open.map(|r| r.status()).ok(), Some(reqwest::StatusCode::SERVICE_UNAVAILABLE));
        let rest = tokio::time::timeout(Duration::from_secs(2), streaming.chunk()).await.expect("cut off");
        assert!(rest.is_err(), "the body ends with an error: {rest:?}");
    }

    /// Scenario: App shutdown with an open stream (the real `GET /changes`, with a short wait in place of five
    /// seconds).
    #[tokio::test]
    async fn an_open_change_stream_gets_the_drain_after_the_node_stopped_then_is_cut_before_the_store_closes() {
        let store = Store::memory();
        let system = ActorSystem::builder(store.clone()).build();
        system.start().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let stop = Stop::watch(async move {
            let _ = rx.await;
        });
        let drain = Duration::from_millis(300);
        let serving = {
            let (system, store) = (system.clone(), store.clone());
            let routes = actor_api(system.clone());
            tokio::spawn(async move {
                let tasks = Tasks::start(Vec::new(), &system, &stop).await;
                serve_and_close(listener, routes, &system, &store, tasks, Duration::from_secs(30), drain).await
            })
        };
        let mut stream = reqwest::get(format!("http://{address}/changes")).await.unwrap();
        assert_eq!(stream.headers()["content-type"], "text/event-stream");
        let first = stream.chunk().await.unwrap().unwrap();
        assert_eq!(&first[..], b"event: resync\ndata: {}\n\n");

        let started = Instant::now();
        tx.send(()).unwrap();
        while system.state() != cicada::runtime::NodeState::Stopped {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let node_stopped = started.elapsed();
        // The node has stopped, the store is still open: the stream stays open during the drain.
        let open = tokio::time::timeout(Duration::from_millis(50), stream.chunk()).await;
        assert!(open.is_err(), "neither ended nor cut yet: {open:?}");
        let rest = tokio::time::timeout(Duration::from_secs(2), stream.chunk()).await.expect("cut off");
        let cut = started.elapsed();
        assert!(rest.is_err(), "the body ends with a transport error, not a final event: {rest:?}");
        assert!(node_stopped < drain && cut >= drain && cut < Duration::from_secs(2), "{node_stopped:?}, {cut:?}");
        serving.await.unwrap().expect("served");
        assert!(started.elapsed() < Duration::from_secs(2), "the open stream added no wait beyond the drain");
        assert!(system.now().await.is_err(), "the store is closed after the cut");
        assert!(futures::StreamExt::next(&mut system.changes("")).await.is_none());
    }
}

/// The worker over HTTP on in-process sources, on virtual time: the wait for the server, the start, the stop and the
/// end.
#[cfg(test)]
mod worker_tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use cicada::actor::contract_of;
    use cicada::address::TypeName;
    use cicada::contract::Contract;
    use cicada::prelude::*;
    use cicada::runtime::{
        CompleteTurnResult, LeaseTurnsResult, RegisterContractResult, RenewTurnResult, TurnReport, TurnSource,
    };
    use cicada::store::{MessageId, NodeId, TypeDef};
    use cicada_client::ActorClient;
    use serde::{Deserialize, Serialize};
    use tokio::time::Instant;

    use super::*;
    use crate::App;
    use crate::config::Target;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Tick {
        id: String,
    }

    impl ActorRef for Tick {
        const TYPE: &'static str = "tick";
        type Message = TickMsg;
    }

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum TickMsg {
        Tick,
    }

    struct TickActor;

    impl Actor for TickActor {
        type Ref = Tick;
        type State = NoState;

        async fn handle(&self, _: &mut Turn<Self>, _: TickMsg) -> HandlerResult {
            Ok(())
        }
    }

    /// Type `tick` with another contract.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct OtherTick {
        id: String,
    }

    impl ActorRef for OtherTick {
        const TYPE: &'static str = "tick";
        type Message = OtherTickMsg;
    }

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum OtherTickMsg {
        Tock,
    }

    struct OtherTickActor;

    impl Actor for OtherTickActor {
        type Ref = OtherTick;
        type State = NoState;

        async fn handle(&self, _: &mut Turn<Self>, _: OtherTickMsg) -> HandlerResult {
            Ok(())
        }
    }

    type Answer<T> = BoxFuture<'static, Result<T, SourceError>>;

    /// An in-process node as the source, whose type listing, registration and leases a test can replace; it records
    /// each call with the time it began.
    struct Fake {
        system: ActorSystem,
        list: Box<dyn Fn(usize) -> Answer<Vec<TypeDef>> + Send + Sync>,
        register: Option<Box<dyn Fn() -> Answer<RegisterContractResult> + Send + Sync>>,
        lease_error: Option<SourceError>,
        calls: Mutex<Vec<(&'static str, Instant)>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                system: ActorSystem::builder(Store::memory()).build(),
                list: Box::new(|_| listed()),
                register: None,
                lease_error: None,
                calls: Mutex::new(Vec::new()),
            }
        }

        /// Type listings answered by `answer` of the number of the call, from 0.
        fn listing(mut self, answer: impl Fn(usize) -> Answer<Vec<TypeDef>> + Send + Sync + 'static) -> Self {
            self.list = Box::new(answer);
            self
        }

        fn registering(mut self, answer: impl Fn() -> Answer<RegisterContractResult> + Send + Sync + 'static) -> Self {
            self.register = Some(Box::new(answer));
            self
        }

        fn record(&self, call: &'static str) -> usize {
            let mut calls = self.calls.lock().unwrap();
            calls.push((call, Instant::now()));
            calls.iter().filter(|(c, _)| *c == call).count() - 1
        }

        fn times(&self, call: &str) -> Vec<Instant> {
            self.calls.lock().unwrap().iter().filter(|(c, _)| *c == call).map(|(_, at)| *at).collect()
        }

        fn count(&self, call: &str) -> usize {
            self.times(call).len()
        }
    }

    #[async_trait]
    impl TurnSource for Fake {
        async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
            self.record("register");
            match &self.register {
                Some(answer) => answer().await,
                None => TurnSource::register_contract(&self.system, contract).await,
            }
        }

        async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
            let number = self.record("list");
            (self.list)(number).await
        }

        async fn lease_turns(
            &self,
            owner: &NodeId,
            types: &std::collections::BTreeMap<TypeName, String>,
            limit: usize,
        ) -> Result<LeaseTurnsResult, SourceError> {
            self.record("lease");
            match &self.lease_error {
                Some(e) => Err(e.clone()),
                None => TurnSource::lease_turns(&self.system, owner, types, limit).await,
            }
        }

        async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
            TurnSource::renew_turn(&self.system, actor, epoch).await
        }

        async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
            TurnSource::release_turn(&self.system, actor, epoch).await
        }

        async fn complete_turn(
            &self,
            actor: &ActorAddress,
            epoch: i64,
            message_id: &MessageId,
            report: TurnReport,
        ) -> Result<CompleteTurnResult, SourceError> {
            TurnSource::complete_turn(&self.system, actor, epoch, message_id, report).await
        }
    }

    fn unavailable<T: Send + 'static>() -> Answer<T> {
        Box::pin(async { Err(SourceError::Unavailable("connection refused".into())) })
    }

    fn listed() -> Answer<Vec<TypeDef>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn never() -> Stop {
        Stop::watch(std::future::pending())
    }

    fn stop_after(after: Duration) -> Stop {
        Stop::watch(tokio::time::sleep(after))
    }

    fn parts() -> Parts {
        App::new().host(TickActor).into_parts()
    }

    fn settings() -> HttpSettings {
        let worker = WorkerConfig {
            poll_interval: Duration::from_millis(50),
            shutdown_grace_period: Duration::from_secs(1),
            ..WorkerConfig::default()
        };
        HttpSettings { types: vec!["tick"], worker }
    }

    /// Scenario: The server comes up late.
    #[tokio::test(start_paused = true)]
    async fn a_server_that_comes_up_late_is_waited_for() {
        let fake = Fake::new().listing(|n| if n < 5 { unavailable() } else { listed() });
        let started = Instant::now();
        assert_eq!(wait_for_server(&fake, &never()).await, Ok(true));
        assert_eq!(started.elapsed(), Duration::from_secs(5));
        assert_eq!(fake.count("list"), 6);
        assert_eq!(fake.count("register") + fake.count("lease"), 0, "a probe runs nothing");
    }

    /// Scenario: Retry horizon: no try at or after a minute from the first, also after a wait.
    #[tokio::test(start_paused = true)]
    async fn a_server_that_stays_unavailable_gets_no_try_after_a_minute() {
        let fake = Fake::new().listing(|_| unavailable());
        let started = Instant::now();
        let error = wait_for_server(&fake, &never()).await.unwrap_err();
        assert!(matches!(error, SourceError::Unavailable(_)), "{error}");
        let times = fake.times("list");
        assert_eq!(times.len(), 60, "one try a second");
        assert_eq!(times.last().unwrap().duration_since(started), Duration::from_secs(59));
        assert_eq!(started.elapsed(), SERVER_WAIT, "the wait after the last try reaches the minute: no try then");
    }

    /// Scenario: Retry horizon: a try that began before the minute ends after it.
    #[tokio::test(start_paused = true)]
    async fn the_last_try_may_end_after_the_minute() {
        let fake = Fake::new().listing(|_| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(25)).await;
                Err(SourceError::Unavailable("timed out".into()))
            })
        });
        let started = Instant::now();
        assert!(wait_for_server(&fake, &never()).await.is_err());
        let began: Vec<Duration> = fake.times("list").iter().map(|t| t.duration_since(started)).collect();
        assert_eq!(began, [Duration::ZERO, Duration::from_secs(26), Duration::from_secs(52)]);
        assert_eq!(started.elapsed(), Duration::from_secs(77), "the third try ended after the minute");
    }

    /// Scenario: Permanent refusal or stop: a refusal fails at once.
    #[tokio::test(start_paused = true)]
    async fn a_permanent_refusal_fails_at_once() {
        for refusal in [SourceError::Invalid("bad request".into()), SourceError::Fatal("HTTP 401".into())] {
            let answer = refusal.clone();
            let fake = Fake::new().listing(move |_| {
                let answer = answer.clone();
                Box::pin(async move { Err(answer) })
            });
            let started = Instant::now();
            assert_eq!(wait_for_server(&fake, &never()).await, Err(refusal));
            assert_eq!((fake.count("list"), started.elapsed()), (1, Duration::ZERO));
        }
    }

    /// Scenario: Permanent refusal or stop: a stop interrupts a try that hangs, and a wait.
    #[tokio::test(start_paused = true)]
    async fn a_stop_interrupts_a_hung_try_and_a_wait() {
        let hung = Fake::new().listing(|_| Box::pin(std::future::pending()));
        let started = Instant::now();
        assert_eq!(wait_for_server(&hung, &stop_after(Duration::from_secs(3))).await, Ok(false));
        assert_eq!(started.elapsed(), Duration::from_secs(3));

        let away = Fake::new().listing(|_| unavailable());
        let started = Instant::now();
        assert_eq!(wait_for_server(&away, &stop_after(Duration::from_millis(2500))).await, Ok(false));
        assert_eq!(started.elapsed(), Duration::from_millis(2500));
        assert_eq!(away.count("list"), 3, "at 0, 1 and 2 s");
    }

    /// Scenario: Permanent refusal or stop: a stop before the server answered starts no worker.
    #[tokio::test(start_paused = true)]
    async fn a_stop_while_the_server_is_waited_for_starts_no_worker() {
        let fake = Arc::new(Fake::new().listing(|_| unavailable()));
        http_worker(parts(), settings(), fake.clone(), stop_after(Duration::from_millis(1500))).await.unwrap();
        assert_eq!(fake.count("register") + fake.count("lease"), 0);
    }

    /// Scenario: Stop during registration.
    #[tokio::test(start_paused = true)]
    async fn a_stop_during_registration_stops_the_worker_before_any_lease() {
        let fake = Arc::new(Fake::new().registering(|| Box::pin(std::future::pending())));
        let started = Instant::now();
        http_worker(parts(), settings(), fake.clone(), stop_after(Duration::from_secs(2))).await.expect("success");
        assert_eq!(started.elapsed(), Duration::from_secs(2));
        assert_eq!((fake.count("register"), fake.count("lease")), (1, 0));
    }

    /// Scenario: Registration fails after successful preflight: a conflict, with its diagnostics.
    #[tokio::test(start_paused = true)]
    async fn a_conflicting_contract_fails_the_start_with_its_diagnostics() {
        let fake = Fake::new();
        fake.system.register_contract(&contract_of::<OtherTickActor>().unwrap()).await.unwrap();
        let fake = Arc::new(fake);
        let error = http_worker(parts(), settings(), fake.clone(), never()).await.unwrap_err();
        assert_eq!(error.exit_code(), 1);
        let text = error.to_string();
        assert!(text.starts_with("the worker cannot start") && text.contains("tick"), "{text}");
        assert_eq!((fake.count("register"), fake.count("lease")), (1, 0));
    }

    /// Scenario: Registration fails after successful preflight: a temporary failure is not tried again.
    #[tokio::test(start_paused = true)]
    async fn an_unavailable_registration_fails_the_start_without_a_retry() {
        let fake = Arc::new(Fake::new().registering(unavailable));
        let error = http_worker(parts(), settings(), fake.clone(), never()).await.unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("connection refused"), "{error}");
        assert_eq!((fake.count("list"), fake.count("register"), fake.count("lease")), (1, 1, 0));
    }

    /// Scenario: A stopped worker and a failed worker: a stop of a healthy worker succeeds.
    #[tokio::test(start_paused = true)]
    async fn a_stop_of_a_healthy_worker_succeeds() {
        let fake = Arc::new(Fake::new());
        http_worker(parts(), settings(), fake.clone(), stop_after(Duration::from_secs(2))).await.expect("exit 0");
        assert!(fake.count("lease") >= 2, "it leased until the stop");
    }

    #[tokio::test(start_paused = true)]
    async fn worker_ready_runs_once_after_a_retry_and_registration_with_the_settled_settings() {
        let fake = Arc::new(Fake::new().listing(|call| if call == 0 { unavailable() } else { listed() }));
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (stop, stopped) = signal();
        let source = fake.clone();
        let parts = App::new()
            .host(TickActor)
            .on_worker_ready(move |ready| {
                assert_eq!(source.count("list"), 2);
                assert_eq!(source.count("register"), 1);
                tx.send(ready).unwrap();
            })
            .into_parts();
        let mut settings = settings();
        settings.worker.shutdown_cancel_timeout = Duration::from_secs(3);
        let worker = tokio::spawn(http_worker(parts, settings, fake, stopped));
        assert_eq!(
            rx.await.unwrap(),
            WorkerReady { grace_period: Duration::from_secs(1), cancel_timeout: Duration::from_secs(3) }
        );
        stop.send(()).unwrap();
        worker.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_worker_ready_hook_is_skipped_when_registration_fails() {
        let parts = App::new().host(TickActor).on_worker_ready(|_| panic!("the worker did not start")).into_parts();
        let fake = Arc::new(Fake::new().registering(unavailable));
        let error = http_worker(parts, settings(), fake, never()).await.unwrap_err();
        assert!(error.to_string().starts_with("the worker cannot start"), "{error}");
    }

    /// Scenario: A stopped worker and a failed worker: a worker that stops by itself fails the command.
    #[tokio::test(start_paused = true)]
    async fn a_worker_that_stops_by_itself_fails_the_command() {
        let fake = Fake { lease_error: Some(SourceError::Fatal("Leasing turns: HTTP 307".into())), ..Fake::new() };
        let error = http_worker(parts(), settings(), Arc::new(fake), never()).await.unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().starts_with("the worker stopped") && error.to_string().contains("307"), "{error}");
    }

    /// Scenario: A stopped worker and a failed worker: a stop that meets a failure already recorded keeps it.
    #[tokio::test(start_paused = true)]
    async fn a_failure_recorded_before_a_stop_still_fails_the_command() {
        let fake = Fake { lease_error: Some(SourceError::Invalid("refused".into())), ..Fake::new() };
        let worker = build_worker(parts().types, Vec::new(), &["tick"], settings().worker, Arc::new(fake));
        worker.start().await.unwrap();
        assert!(worker.stopped().await.is_some(), "it stopped by itself");
        let stop = Stop::watch(async {});
        while !stop.is_set() {
            tokio::task::yield_now().await;
        }
        // Both are ready; the stop is taken first.
        let error = run_until_stopped(&worker, &stop).await.unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("refused"), "{error}");
    }

    // ---- over HTTP, on virtual time ----
    //
    // The client of `Target::worker_client` against a real socket, every call held at a barrier that stops the paused
    // clock while it is in flight (the harness of `cicada-client`'s `remote_worker` tests): virtual time moves only
    // between requests, so timing assertions are exact.

    /// Holds the paused clock while a call is in flight: a blocking task stops tokio's auto-advance on a
    /// current-thread runtime until the call ends.
    struct InFlight(#[allow(dead_code)] std::sync::mpsc::Sender<()>);

    fn in_flight() -> InFlight {
        let (held, released) = std::sync::mpsc::channel::<()>();
        tokio::task::spawn_blocking(move || {
            let _ = released.recv();
        });
        InFlight(held)
    }

    /// A source with every call held at the barrier of [`in_flight`].
    struct Held(ActorClient);

    #[async_trait]
    impl TurnSource for Held {
        async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
            let _held = in_flight();
            self.0.register_contract(contract).await
        }

        async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
            let _held = in_flight();
            self.0.list_types().await
        }

        async fn lease_turns(
            &self,
            owner: &NodeId,
            types: &std::collections::BTreeMap<TypeName, String>,
            limit: usize,
        ) -> Result<LeaseTurnsResult, SourceError> {
            let _held = in_flight();
            self.0.lease_turns(owner, types, limit).await
        }

        async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
            let _held = in_flight();
            self.0.renew_turn(actor, epoch).await
        }

        async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
            let _held = in_flight();
            self.0.release_turn(actor, epoch).await
        }

        async fn complete_turn(
            &self,
            actor: &ActorAddress,
            epoch: i64,
            message_id: &MessageId,
            report: TurnReport,
        ) -> Result<CompleteTurnResult, SourceError> {
            let _held = in_flight();
            self.0.complete_turn(actor, epoch, message_id, report).await
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Gated {
        id: String,
    }

    impl ActorRef for Gated {
        const TYPE: &'static str = "gated";
        type Message = GatedMsg;
    }

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type")]
    enum GatedMsg {
        /// Tells the test it began, then waits until the gate opens.
        Wait,
    }

    #[derive(Clone)]
    struct Gate {
        began: Arc<tokio::sync::Notify>,
        open: tokio::sync::watch::Sender<bool>,
    }

    struct GatedActor(Gate);

    impl Actor for GatedActor {
        type Ref = Gated;
        type State = NoState;

        async fn handle(&self, _: &mut Turn<Self>, _: GatedMsg) -> HandlerResult {
            self.0.began.notify_one();
            let _ = self.0.open.subscribe().wait_for(|open| *open).await;
            Ok(())
        }
    }

    /// A request the server saw: method, path, `Authorization` and `X-Tenant`.
    type SeenRequest = (String, String, Option<String>, Option<String>);

    /// A node that hosts nothing, on an in-memory store on tokio's clock (store time is virtual time), with a lease of
    /// 3 s, serving the API on a real socket behind a layer that records each request and answers `401` without
    /// `Authorization: Bearer s3cret` and `X-Tenant: ops`.
    struct Server {
        system: ActorSystem,
        base: String,
        seen: Arc<Mutex<Vec<SeenRequest>>>,
    }

    impl Server {
        async fn start() -> Self {
            let store = Store::memory_on(Arc::new(cicada_test::TokioClock::new(cicada_test::default_start())));
            let config = cicada::runtime::NodeConfig { lease_ttl: Duration::from_secs(3), ..Default::default() };
            let system = ActorSystem::builder(store).config(config).build();
            let seen: Arc<Mutex<Vec<SeenRequest>>> = Arc::default();
            let record = seen.clone();
            let routes = actor_api(system.clone()).layer(axum::middleware::from_fn(move |request: Request, next: Next| {
                let header = |name: &str| request.headers().get(name).map(|v| v.to_str().unwrap().to_owned());
                let request_seen =
                    (request.method().to_string(), request.uri().path().to_owned(), header("authorization"), header("x-tenant"));
                let allowed = request_seen.2.as_deref() == Some("Bearer s3cret") && request_seen.3.as_deref() == Some("ops");
                record.lock().unwrap().push(request_seen);
                async move {
                    if allowed { next.run(request).await } else { StatusCode::UNAUTHORIZED.into_response() }
                }
            }));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move { axum::serve(listener, routes).await });
            Self { system, base, seen }
        }

        /// The source of a worker over HTTP with `token` and the header `X-Tenant: ops`, held at the barrier.
        fn source(&self, token: &str) -> Arc<dyn TurnSource> {
            let target = Target {
                context: None,
                server: self.base.clone(),
                token: Some(token.to_owned()),
                headers: std::collections::BTreeMap::from([("X-Tenant".to_owned(), "ops".to_owned())]),
            };
            Arc::new(Held(target.worker_client().unwrap()))
        }

        fn count(&self, method: &str, path: &str) -> usize {
            self.seen.lock().unwrap().iter().filter(|s| s.0 == method && s.1 == path).count()
        }

        /// Waits until `gated` is registered, then sends it `Wait`.
        async fn send_wait(&self) -> MessageId {
            while !self.system.types().await.unwrap().iter().any(|t| t.definition.name.as_str() == "gated") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let gated = Gated { id: "g".into() };
            self.system.create(&gated, &NoState {}).await.unwrap();
            match self.system.send(&gated, GatedMsg::Wait).await.unwrap() {
                cicada::SendResult::Accepted(m) => m.id,
                other => panic!("not accepted: {other:?}"),
            }
        }

        async fn finished(&self, id: &MessageId) -> Option<cicada::store::OutcomeStatus> {
            match self.system.status(id).await.unwrap()? {
                cicada::store::MessageStatus::Finished(o) => Some(o.status),
                _ => None,
            }
        }
    }

    fn gated_parts(gate: &Gate) -> Parts {
        App::new().host(GatedActor(gate.clone())).into_parts()
    }

    fn gated_settings() -> HttpSettings {
        let worker = WorkerConfig {
            poll_interval: Duration::from_millis(50),
            shutdown_grace_period: Duration::from_secs(30),
            ..WorkerConfig::default()
        };
        HttpSettings { types: vec!["gated"], worker }
    }

    fn gate() -> Gate {
        Gate { began: Arc::default(), open: tokio::sync::watch::channel(false).0 }
    }

    /// A stop that completes when `tx` sends or is dropped.
    fn signal() -> (tokio::sync::oneshot::Sender<()>, Stop) {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        (
            tx,
            Stop::watch(async move {
                let _ = rx.await;
            }),
        )
    }

    /// Scenario: Credentials and diagnostics: the token and header reach the type and every lease route, renewals of a
    /// held turn included (a renewal a third of the TTL of 3 s after the lease, then each second).
    #[tokio::test(start_paused = true)]
    async fn over_http_the_token_and_header_reach_every_route_and_a_held_turn_is_renewed() {
        let server = Server::start().await;
        let gate = gate();
        let (stop, stopped) = signal();
        let worker = tokio::spawn(http_worker(gated_parts(&gate), gated_settings(), server.source("s3cret"), stopped));
        let id = server.send_wait().await;
        gate.began.notified().await;
        tokio::time::sleep(Duration::from_millis(4500)).await;
        assert_eq!(server.count("POST", "/leases/renew"), 4, "at 1, 2, 3 and 4 s");
        assert_eq!(server.finished(&id).await, None, "held for longer than the TTL");
        gate.open.send_replace(true);
        while server.finished(&id).await.is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(server.finished(&id).await, Some(cicada::store::OutcomeStatus::Done));
        stop.send(()).unwrap();
        worker.await.unwrap().expect("exit code 0");
        for (method, path) in
            [("GET", "/types"), ("PUT", "/types/gated"), ("POST", "/leases"), ("POST", "/leases/complete")]
        {
            assert!(server.count(method, path) >= 1, "{method} {path}");
        }
        let seen = server.seen.lock().unwrap().clone();
        assert!(
            seen.iter().all(|s| s.2.as_deref() == Some("Bearer s3cret") && s.3.as_deref() == Some("ops")),
            "{seen:?}"
        );
    }

    /// Scenario: Permanent refusal or stop: a refused token fails at once, on the first try.
    #[tokio::test(start_paused = true)]
    async fn over_http_a_refused_token_fails_at_once() {
        let server = Server::start().await;
        let started = Instant::now();
        let error = http_worker(gated_parts(&gate()), gated_settings(), server.source("wrong"), never()).await;
        let error = error.unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(
            error.to_string().starts_with("the server is not ready") && error.to_string().contains("401"),
            "{error}"
        );
        assert!(!error.to_string().contains("wrong"), "the token is not printed: {error}");
        assert_eq!(started.elapsed(), Duration::ZERO, "no wait");
        assert_eq!(server.seen.lock().unwrap().len(), 1, "one try");
    }

    /// Scenario: A stopped worker and a failed worker: a stop takes effect at once (no more lease requests) and waits
    /// for the running turn within the grace period of 30 s; the finished handler is completed, not released.
    #[tokio::test(start_paused = true)]
    async fn over_http_a_stop_waits_for_the_running_turn_within_the_grace_period() {
        let server = Server::start().await;
        let gate = gate();
        let (stop, stopped) = signal();
        let mut worker =
            tokio::spawn(http_worker(gated_parts(&gate), gated_settings(), server.source("s3cret"), stopped));
        let id = server.send_wait().await;
        gate.began.notified().await;
        stop.send(()).unwrap();
        let signalled = Instant::now();
        assert!(tokio::time::timeout(Duration::from_secs(5), &mut worker).await.is_err(), "still waiting at 5 s");
        let leases = server.count("POST", "/leases");
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(server.count("POST", "/leases"), leases, "the stop was taken: no lease request since");
        gate.open.send_replace(true);
        worker.await.unwrap().expect("exit code 0");
        assert_eq!(signalled.elapsed(), Duration::from_secs(10), "it ended when the handler did");
        assert_eq!(server.finished(&id).await, Some(cicada::store::OutcomeStatus::Done));
        assert_eq!(server.count("POST", "/leases/release"), 0);
    }
}
