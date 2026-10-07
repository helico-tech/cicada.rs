//! The nodes of the stack, raw and typed HTTP access to them, and polling with deadlines.

use std::fmt;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use cicada::api::{ActorView, MessageStatusView, OutcomeView};
use cicada::store::MessageId;
use cicada::{ActorAddress, ActorRef};
use cicada_client::ActorClient;
use cicada_sample_service::e2e::Effect;
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::docker::compose;

/// The plain HTTP client of the tests: bounded connect and request timeouts, no authentication.
pub fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| client(None))
}

fn client(token: Option<&str>) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(token) = token {
        headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {token}").parse().expect("a valid header"));
    }
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(20))
        .default_headers(headers)
        .build()
        .expect("an HTTP client")
}

/// A response kept as status, body text and content type, for exact status assertions.
#[derive(Debug, Clone)]
pub struct RawResponse {
    /// The HTTP status.
    pub status: u16,
    /// The body as text.
    pub body: String,
    /// The `Content-Type` header.
    pub content_type: Option<String>,
}

impl RawResponse {
    /// The body as JSON.
    ///
    /// # Panics
    ///
    /// When the body is not JSON.
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {self}"))
    }

    /// The string at `pointer` (RFC 6901, e.g. `/type`) of the JSON body, if any.
    pub fn text(&self, pointer: &str) -> Option<String> {
        self.json().pointer(pointer).and_then(Value::as_str).map(str::to_owned)
    }
}

impl fmt::Display for RawResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.status, self.body)
    }
}

/// One sample node, reachable on the host at `localhost:<port>`.
#[derive(Debug, Clone)]
pub struct Node {
    /// The node id (`NODE_ID`), which is also its compose service or container name.
    pub id: String,
    /// The host port.
    pub port: u16,
    /// Whether it runs turns (false for the send-only node).
    pub hosting: bool,
    /// The bearer token its API requires, if any.
    pub token: Option<String>,
    /// `http://localhost:<port>`.
    pub base_url: String,
    /// The typed client pointed at this node (with the token).
    pub client: ActorClient,
}

impl Node {
    /// The node `id` on host port `port`.
    pub fn new(id: &str, port: u16, hosting: bool, token: Option<&str>) -> Self {
        let base_url = format!("http://localhost:{port}");
        let client = ActorClient::with_client(&base_url, client(token))
            .send_attempts(3)
            .retry_delay(Duration::from_millis(200));
        Self { id: id.to_owned(), port, hosting, token: token.map(str::to_owned), base_url, client }
    }

    /// A raw request with this node's token; `body` is sent as JSON.
    pub async fn request(&self, method: Method, path: &str, body: Option<&str>) -> anyhow::Result<RawResponse> {
        self.request_as(method, path, body, self.token.as_deref()).await
    }

    /// A raw request with the bearer `token` (none when `None`); `body` is sent as JSON.
    pub async fn request_as(
        &self,
        method: Method,
        path: &str,
        body: Option<&str>,
        token: Option<&str>,
    ) -> anyhow::Result<RawResponse> {
        let mut request = http().request(method.clone(), format!("{}{path}", self.base_url));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.header(reqwest::header::CONTENT_TYPE, "application/json").body(body.to_owned());
        }
        let response = request.send().await.with_context(|| format!("{method} {path} on {self}"))?;
        let status = response.status().as_u16();
        let content_type =
            response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let body = response.text().await?;
        Ok(RawResponse { status, body, content_type })
    }

    /// `GET path`.
    pub async fn get(&self, path: &str) -> RawResponse {
        self.request(Method::GET, path, None).await.unwrap()
    }

    /// `PUT path` with a JSON body.
    pub async fn put(&self, path: &str, body: &str) -> RawResponse {
        self.request(Method::PUT, path, Some(body)).await.unwrap()
    }

    /// `POST path` with a JSON body.
    pub async fn post(&self, path: &str, body: &str) -> RawResponse {
        self.request(Method::POST, path, Some(body)).await.unwrap()
    }

    /// `DELETE path`.
    pub async fn delete(&self, path: &str) -> RawResponse {
        self.request(Method::DELETE, path, None).await.unwrap()
    }

    /// True when `GET /health` answers 200.
    pub async fn healthy(&self) -> bool {
        matches!(self.request(Method::GET, "/health", None).await, Ok(r) if r.status == 200)
    }

    /// This node's in-memory side-effect log for the actor `uri` (`GET /e2e/effects`).
    pub async fn effects(&self, uri: &str) -> anyhow::Result<Vec<Effect>> {
        let mut url = reqwest::Url::parse(&format!("{}/e2e/effects", self.base_url))?;
        url.query_pairs_mut().append_pair("actor", uri);
        let mut request = http().get(url);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        if response.status() != 200 {
            bail!("effects on {self}: {}", response.status());
        }
        Ok(response.json().await?)
    }

    /// The snapshot of `r`.
    ///
    /// # Panics
    ///
    /// When the call fails or the instance does not exist.
    pub async fn view<R: ActorRef>(&self, r: &R) -> ActorView {
        let address = address(r);
        self.client.get(r).await.unwrap_or_else(|e| panic!("get {address} on {self}: {e}")).expect("the instance")
    }

    /// The state of `r` decoded as `S`.
    ///
    /// # Panics
    ///
    /// When the call fails or the instance does not exist.
    pub async fn state<S: DeserializeOwned, R: ActorRef>(&self, r: &R) -> S {
        let address = address(r);
        self.client.state(r).await.unwrap_or_else(|e| panic!("state of {address} on {self}: {e}")).expect("the instance")
    }

    /// The state of `r` decoded as `S`; an error when the call fails or the instance does not exist (for
    /// [`eventually`]).
    pub async fn try_state<S: DeserializeOwned, R: ActorRef>(&self, r: &R) -> anyhow::Result<S> {
        self.client.state(r).await?.with_context(|| format!("{} does not exist", address(r)))
    }

    /// The status of message `id`; an error when the call fails or the message is unknown (for [`eventually`]).
    pub async fn try_status(&self, id: &MessageId) -> anyhow::Result<MessageStatusView> {
        self.client.status(id).await?.with_context(|| format!("unknown message {id}"))
    }

    /// The failed messages of instances whose URI starts with `prefix` (for [`eventually`]).
    pub async fn try_failures(&self, prefix: &str) -> anyhow::Result<Vec<OutcomeView>> {
        Ok(self.client.failures(prefix, None, None, None).await?.items)
    }

    /// The status of message `id`.
    ///
    /// # Panics
    ///
    /// When the call fails or the message is unknown.
    pub async fn status(&self, id: &MessageId) -> MessageStatusView {
        self.client.status(id).await.unwrap_or_else(|e| panic!("status of {id} on {self}: {e}")).expect("a known message")
    }

    /// The failed messages of instances whose URI starts with `prefix`.
    ///
    /// # Panics
    ///
    /// When the call fails.
    pub async fn failures(&self, prefix: &str) -> Vec<OutcomeView> {
        self.client.failures(prefix, None, None, None).await.unwrap_or_else(|e| panic!("failures on {self}: {e}")).items
    }

    /// Waits up to 60 s until message `id` has an outcome and returns its status.
    pub async fn await_finished(&self, id: &MessageId) -> MessageStatusView {
        self.await_finished_within(id, Duration::from_secs(60)).await
    }

    /// Waits until message `id` has an outcome and returns its status.
    pub async fn await_finished_within(&self, id: &MessageId, timeout: Duration) -> MessageStatusView {
        eventually(&format!("message {id} finished (asking {self})"), timeout, async || {
            Ok(self.client.status(id).await?.filter(|s| s.status.is_finished()))
        })
        .await
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id)
    }
}

/// The address of a ref (`actor://…`).
pub fn address<R: ActorRef>(r: &R) -> ActorAddress {
    r.address().expect("a valid ref")
}

/// The ref's URI string, e.g. for prefix queries.
pub fn uri<R: ActorRef>(r: &R) -> String {
    address(r).to_string()
}

/// The HTTP path of an instance: `actor://a/b` → `/actors/a/b`.
pub fn path<R: ActorRef>(r: &R) -> String {
    address(r).to_http_path()
}

/// Polls `check` every 200 ms until it returns `Some`, for at most `timeout`; see [`eventually_every`].
pub async fn eventually<T>(what: &str, timeout: Duration, check: impl AsyncFnMut() -> anyhow::Result<Option<T>>) -> T {
    eventually_every(what, timeout, Duration::from_millis(200), check).await
}

/// Polls `check` every `interval` until it returns `Some`, for at most `timeout`. Errors (a node that is down, a
/// refused connection) count as "not yet"; the last one is reported on timeout.
///
/// # Panics
///
/// On timeout.
pub async fn eventually_every<T>(
    what: &str,
    timeout: Duration,
    interval: Duration,
    mut check: impl AsyncFnMut() -> anyhow::Result<Option<T>>,
) -> T {
    let started = Instant::now();
    let mut last_error = None;
    loop {
        match check().await {
            Ok(Some(value)) => return value,
            Ok(None) => {}
            Err(e) => last_error = Some(e),
        }
        if started.elapsed() > timeout {
            match last_error {
                Some(e) => panic!("Timed out after {timeout:?} waiting for {what}; last error: {e:#}"),
                None => panic!("Timed out after {timeout:?} waiting for {what}"),
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// The PostgreSQL cluster of `e2e/docker-compose.yml`.
#[derive(Debug)]
pub struct Cluster {
    /// `node-1`, host port 9081.
    pub node1: Node,
    /// `node-2`, host port 9082.
    pub node2: Node,
    /// `node-3`, host port 9083.
    pub node3: Node,
    /// `send-only`, host port 9084: hosts no types.
    pub send_only: Node,
}

/// The cluster.
pub fn cluster() -> &'static Cluster {
    static CLUSTER: OnceLock<Cluster> = OnceLock::new();
    CLUSTER.get_or_init(|| Cluster {
        node1: Node::new("node-1", 9081, true, None),
        node2: Node::new("node-2", 9082, true, None),
        node3: Node::new("node-3", 9083, true, None),
        send_only: Node::new("send-only", 9084, false, None),
    })
}

impl Cluster {
    /// The three hosting nodes.
    pub fn hosting_nodes(&self) -> [&Node; 3] {
        [&self.node1, &self.node2, &self.node3]
    }

    /// All four nodes: the hosting ones, then the send-only one.
    pub fn nodes(&self) -> [&Node; 4] {
        [&self.node1, &self.node2, &self.node3, &self.send_only]
    }

    /// The node with id `id`.
    pub fn node(&self, id: &str) -> &Node {
        self.nodes().into_iter().find(|n| n.id == id).unwrap_or_else(|| panic!("no node {id}"))
    }

    /// Waits until every node answers `/health` with 200.
    pub async fn await_healthy(&self, timeout: Duration) {
        for node in self.nodes() {
            eventually(&format!("{node} healthy"), timeout, async || Ok(node.healthy().await.then_some(()))).await;
        }
    }

    /// Brings the cluster back to all-up after a fault test: unpauses, starts what was killed or stopped, and waits
    /// for every node to be healthy.
    pub async fn restore(&self) {
        for node in self.hosting_nodes() {
            compose::unpause(&node.id).await;
        }
        compose::unpause(compose::POSTGRES).await;
        let _ = compose::start(compose::POSTGRES, false).await;
        // `up --wait` fails at once on a dependency whose health check still says "unhealthy" (a node that lost its
        // store recovers by itself, but its next health check is up to 5 s away): give them that time first.
        let settled = tokio::time::timeout(Duration::from_secs(60), async {
            while !compose::unhealthy().await.map(|u| u.is_empty()).unwrap_or(false) {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        let _ = settled.await;
        compose::up().await.expect("compose up");
        self.await_healthy(Duration::from_secs(90)).await;
    }
}

/// A suffix that keeps instance ids of one run apart from any other: the start time in base 36.
pub fn run_id() -> &'static str {
    static RUN_ID: OnceLock<String> = OnceLock::new();
    RUN_ID.get_or_init(|| {
        let mut millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        let mut digits = Vec::new();
        while millis > 0 {
            digits.push(char::from_digit((millis % 36) as u32, 36).unwrap());
            millis /= 36;
        }
        digits.iter().rev().collect()
    })
}
