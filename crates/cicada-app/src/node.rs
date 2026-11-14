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
use futures::future::BoxFuture;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::AppError;
use crate::config::Target;
use crate::settings::{HttpSettings, Settings, StoreSpec, check_built_in};

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

/// What the host gave `App` for a node.
pub(crate) struct Parts {
    pub types: Vec<HostedType>,
    pub observers: Vec<Arc<dyn TurnObserver>>,
    pub http: Vec<HttpFn>,
    pub on_start: Vec<OnStartFn>,
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
    let address = listener.local_addr().map_err(|e| AppError::Failed(e.into()))?;
    let written = writeln!(out, "Listening on http://{address}").and_then(|()| out.flush());
    if let Err(e) = written {
        system.stop().await;
        store.close().await;
        return Err(AppError::Failed(anyhow!(e).context("cannot write to the output")));
    }
    tracing::info!("Serving the HTTP API on {address}");
    let served = serve_and_close(listener, routes, &system, &store, &stop, DRAIN).await;
    served.context("the HTTP server failed").map_err(AppError::Failed)
}

/// The end of `serve`: requests until the stop (see [`serve_until_stopped`]), then the node stops (at once when the
/// stop stopped it; else the server failed first) and the store is closed.
async fn serve_and_close(
    listener: TcpListener,
    routes: Router,
    system: &ActorSystem,
    store: &Store,
    stop: &Stop,
    drain: Duration,
) -> std::io::Result<()> {
    let served = serve_until_stopped(listener, routes, system, stop, drain).await;
    system.stop().await;
    store.close().await;
    served
}

/// `worker`: steps 1, 4 and 5 of the start, then work until the stop.
pub(crate) async fn worker(parts: Parts, settings: Settings, stop: Stop) -> Result<(), AppError> {
    let Some(store) = open_unless_stopped(&settings, &stop).await? else { return Ok(()) };
    let system = build(parts.types, parts.observers, &settings, &store);
    if !start(&system, &store, parts.on_start, &stop).await? {
        return Ok(());
    }
    stop.wait().await;
    system.stop().await;
    store.close().await;
    Ok(())
}

/// A worker over HTTP: waits for the server ([`wait_for_server`]), starts a `Worker` of the selected types on the
/// client of `target`, and runs until the stop or until the worker stops by itself. It opens no store and no listener,
/// and never stops or closes the source.
pub(crate) async fn http_worker(
    parts: Parts,
    settings: HttpSettings,
    target: Target,
    stop: Stop,
) -> Result<(), AppError> {
    let client = target.worker_client().map_err(AppError::Failed)?;
    run_worker(parts, settings, Arc::new(client), stop).await
}

/// [`http_worker`] on any `source`.
async fn run_worker(
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
    check_built_in(&settings.store, &crate::settings::built_in()).map_err(|e| AppError::Failed(anyhow!(e)))?;
    tokio::select! {
        biased;
        () = stop.wait() => Ok(None),
        store = open(settings) => store.map(Some).map_err(AppError::Failed),
    }
}

/// Opens the store of `settings`; PostgreSQL is waited for while it answers "unavailable".
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
                Err(anyhow!(check_built_in(&settings.store, &[]).unwrap_err()))
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
                Err(anyhow!(check_built_in(&settings.store, &[]).unwrap_err()))
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

/// Serves `routes` on `listener` until the stop: the node stops first (it claims no more, running turns get its grace
/// period, and the API keeps answering while `/ready` answers `503`), then the server stops, and requests that are
/// still open get `drain` to end before they are cut off.
async fn serve_until_stopped(
    listener: TcpListener,
    routes: Router,
    system: &ActorSystem,
    stop: &Stop,
    drain: Duration,
) -> std::io::Result<()> {
    let (stopped, node_stopped) = tokio::sync::oneshot::channel::<()>();
    let token = stop.token.clone();
    let node = system.clone();
    let graceful = async move {
        token.cancelled().await;
        node.stop().await;
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
                serve_until_stopped(listener, routes, &system, &stop, Duration::from_millis(300)).await
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
            tokio::spawn(async move { serve_and_close(listener, routes, &system, &store, &stop, drain).await })
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
    use serde::{Deserialize, Serialize};
    use tokio::time::Instant;

    use super::*;

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
        let actor = ActorType::from(TickActor);
        let tick = HostedType { name: "tick", host: Box::new(move |hosting| hosting.host(actor)) };
        Parts { types: vec![tick], observers: Vec::new(), http: Vec::new(), on_start: Vec::new() }
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
        run_worker(parts(), settings(), fake.clone(), stop_after(Duration::from_millis(1500))).await.unwrap();
        assert_eq!(fake.count("register") + fake.count("lease"), 0);
    }

    /// Scenario: Stop during registration.
    #[tokio::test(start_paused = true)]
    async fn a_stop_during_registration_stops_the_worker_before_any_lease() {
        let fake = Arc::new(Fake::new().registering(|| Box::pin(std::future::pending())));
        let started = Instant::now();
        run_worker(parts(), settings(), fake.clone(), stop_after(Duration::from_secs(2))).await.expect("success");
        assert_eq!(started.elapsed(), Duration::from_secs(2));
        assert_eq!((fake.count("register"), fake.count("lease")), (1, 0));
    }

    /// Scenario: Registration fails after successful preflight: a conflict, with its diagnostics.
    #[tokio::test(start_paused = true)]
    async fn a_conflicting_contract_fails_the_start_with_its_diagnostics() {
        let fake = Fake::new();
        fake.system.register_contract(&contract_of::<OtherTickActor>().unwrap()).await.unwrap();
        let fake = Arc::new(fake);
        let error = run_worker(parts(), settings(), fake.clone(), never()).await.unwrap_err();
        assert_eq!(error.exit_code(), 1);
        let text = error.to_string();
        assert!(text.starts_with("the worker cannot start") && text.contains("tick"), "{text}");
        assert_eq!((fake.count("register"), fake.count("lease")), (1, 0));
    }

    /// Scenario: Registration fails after successful preflight: a temporary failure is not tried again.
    #[tokio::test(start_paused = true)]
    async fn an_unavailable_registration_fails_the_start_without_a_retry() {
        let fake = Arc::new(Fake::new().registering(unavailable));
        let error = run_worker(parts(), settings(), fake.clone(), never()).await.unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("connection refused"), "{error}");
        assert_eq!((fake.count("list"), fake.count("register"), fake.count("lease")), (1, 1, 0));
    }

    /// Scenario: A stopped worker and a failed worker: a stop of a healthy worker succeeds.
    #[tokio::test(start_paused = true)]
    async fn a_stop_of_a_healthy_worker_succeeds() {
        let fake = Arc::new(Fake::new());
        run_worker(parts(), settings(), fake.clone(), stop_after(Duration::from_secs(2))).await.expect("exit 0");
        assert!(fake.count("lease") >= 2, "it leased until the stop");
    }

    /// Scenario: A stopped worker and a failed worker: a worker that stops by itself fails the command.
    #[tokio::test(start_paused = true)]
    async fn a_worker_that_stops_by_itself_fails_the_command() {
        let fake = Fake { lease_error: Some(SourceError::Fatal("Leasing turns: HTTP 307".into())), ..Fake::new() };
        let error = run_worker(parts(), settings(), Arc::new(fake), never()).await.unwrap_err();
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
}
