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
use cicada::runtime::ActorSystemBuilder;
use cicada::store::StoreError;
use cicada::{ActorSystem, Store};
use cicada_axum::{actor_api, actor_health};
use futures::future::BoxFuture;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::AppError;
use crate::settings::{Settings, StoreSpec, check_built_in};

/// How long `serve` and `worker` wait for a PostgreSQL store that answers "unavailable", and how often they try.
#[cfg_attr(not(feature = "postgres"), allow(dead_code))]
const STORE_WAIT: Duration = Duration::from_secs(60);
#[cfg_attr(not(feature = "postgres"), allow(dead_code))]
const STORE_RETRY: Duration = Duration::from_secs(1);
/// How long requests that are still open may take to end once the node stopped.
const DRAIN: Duration = Duration::from_secs(5);

/// A type of the binary: its name, and how a node hosts it.
pub(crate) struct HostedType {
    pub name: &'static str,
    pub host: Box<dyn FnOnce(ActorSystemBuilder) -> ActorSystemBuilder + Send>,
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
    let mut builder = ActorSystem::builder(store.clone()).config(settings.node.clone());
    for hosted in types.into_iter().filter(|t| settings.types.contains(&t.name)) {
        builder = (hosted.host)(builder);
    }
    for observer in observers {
        builder = builder.observe(observer);
    }
    builder.build()
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
