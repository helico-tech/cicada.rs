//! Typed client for the Cicada HTTP API (`cicada_axum::actor_api`), for callers
//! outside the node's process.
//!
//! ```no_run
//! # async fn example() -> Result<(), cicada_client::ClientError> {
//! use cicada_client::ActorClient;
//! # use cicada::prelude::*;
//! # #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)] struct Sync { tenant: String }
//! # impl ActorRef for Sync { const TYPE: &'static str = "sync"; type Message = SyncMsg; }
//! # #[derive(Debug, serde::Serialize, serde::Deserialize, JsonSchema)] #[serde(tag = "type")] enum SyncMsg { Run }
//! # #[derive(serde::Serialize)] struct SyncState { source: String }
//!
//! let actors = ActorClient::new("https://sync.internal");
//! actors.create(&Sync { tenant: "acme".into() }, &SyncState { source: "https://api.acme.test/items".into() }).await?;
//! let receipt = actors.send(&Sync { tenant: "acme".into() }, SyncMsg::Run).await?;
//! actors.status(&receipt.id).await?;   // pending, running, done, failed, …
//! # Ok(())
//! # }
//! ```
//!
//! Refs and messages are the same types the actor types are defined with, and the bodies are those of [`cicada::api`],
//! so client and server cannot disagree on the format. Request bodies leave absent optional fields out, so a node that
//! does not know a field yet still accepts a request that does not set it. The [`reqwest::Client`]'s own
//! configuration (default headers for authentication, timeouts, TLS) applies to every request.
//!
//! **Results.** Expected alternatives are values: a missing instance or message is `None`, a create reports whether it
//! [created](CreateActorResponse::created) the instance, a send whether it was a
//! [duplicate](SendReceipt::duplicate), a cancel returns a [`CancelOutcome`]. Every other error response is
//! [`ClientError::Api`] with the server's [`Problem`]; transport failures are [`ClientError::Http`].
//!
//! **Idempotent sends.** Every send carries a client-generated id (unless [`SendOptions::id`] gives one). A send that
//! fails with a transport error or a `503` is retried with the same id, up to [`ActorClient::send_attempts`] attempts,
//! so it is enqueued at most once even when an acknowledgement was lost; the server answers a repeated id with `200`,
//! reported as [`SendReceipt::duplicate`]. To retry later yourself, pass the same id again.

use std::time::Duration;

use cicada::address::{TypeName, percent};
use cicada::api::{
    ActorView, BulkRequest, BulkResponse, CreateActorRequest, CreateActorResponse, MailboxMessageView,
    MessageStatusView, MessageView, NodeDetailView, NodeListView, NodeView, OutcomeView, PageView, Problem,
    PurgeResponse, RunningTurnView, SendRequest, SendResponse, StatsView, TypeListView, TypeView, problem_types,
};
use cicada::store::{MessageId, NodeId, OutcomeStatus, RemoveTypeResult};
use cicada::time::{Timestamp, format_instant};
use cicada::{ActorAddress, ActorRef, SendOptions, message};
use reqwest::{Method, StatusCode, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A failed call.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The API answered with an error: the RFC 9457 problem it sent ([`Problem::problem_type`] is one of
    /// [`problem_types`]), or one made up from the status for a response without a problem body (type
    /// `about:blank`, e.g. a `401` of the host's authentication).
    #[error("{} {}{}", .0.status, .0.title, .0.detail.as_deref().map(|d| format!(": {d}")).unwrap_or_default())]
    Api(Problem),
    /// The request could not be sent or its response not read.
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// A success response whose body is not what the API sends.
    #[error("Unexpected response body: {0}")]
    Body(String),
    /// The request cannot be made: a ref, message or state that cannot be encoded, or an address this client cannot
    /// put into a URL.
    #[error("{0}")]
    Invalid(String),
}

impl ClientError {
    /// The HTTP status of an [`Self::Api`] error.
    pub fn status(&self) -> Option<u16> {
        self.problem().map(|p| p.status)
    }

    /// The problem of an [`Self::Api`] error.
    pub fn problem(&self) -> Option<&Problem> {
        match self {
            ClientError::Api(p) => Some(p),
            _ => None,
        }
    }
}

/// Result of a client call.
pub type ClientResult<T> = Result<T, ClientError>;

/// Acknowledgement of a send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendReceipt {
    /// The message id; query it with [`ActorClient::status`].
    pub id: MessageId,
    /// Effective delivery time (store time); `None` for a duplicate whose original is no longer known.
    pub deliver_at: Option<Timestamp>,
    /// True when the server had already accepted this id (`200`), e.g. on a retried send.
    pub duplicate: bool,
}

/// Result of [`ActorClient::cancel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The message was pending and is now cancelled (`204`).
    Cancelled,
    /// The message is being handled (`409`).
    Running,
    /// The message already has an outcome (`409`).
    AlreadyFinished,
    /// Unknown id, or past retention (`404`).
    NotFound,
}

/// A response with its body read.
struct Answer {
    status: StatusCode,
    text: String,
}

impl Answer {
    fn decode<T: DeserializeOwned>(&self) -> ClientResult<T> {
        serde_json::from_str(&self.text).map_err(|e| ClientError::Body(format!("{e}: {}", self.text)))
    }

    fn ok<T: DeserializeOwned>(&self) -> ClientResult<T> {
        if self.status == StatusCode::OK { self.decode() } else { Err(self.failure()) }
    }

    /// The problem in this error response, or one made up from the status when the body is not a problem.
    fn problem(&self) -> Problem {
        serde_json::from_str(&self.text).unwrap_or_else(|_| Problem {
            problem_type: "about:blank".to_owned(),
            title: self.status.canonical_reason().unwrap_or("Unknown status").to_owned(),
            status: self.status.as_u16(),
            detail: Some(self.text.clone()).filter(|t| !t.trim().is_empty()),
            instance: None,
        })
    }

    fn failure(&self) -> ClientError {
        ClientError::Api(self.problem())
    }

    /// A 404 that is the API's own answer of kind `expected` means "absent" (`None`); any other 404 (e.g. a wrong base
    /// URL) is an error.
    fn absent<T>(&self, expected: &str) -> ClientResult<Option<T>> {
        let problem = self.problem();
        if problem.problem_type == expected { Ok(None) } else { Err(ClientError::Api(problem)) }
    }
}

/// Typed client for the actor HTTP API. Cheap to clone; clones share the connection pool.
#[derive(Debug, Clone)]
pub struct ActorClient {
    base: String,
    http: reqwest::Client,
    send_attempts: u32,
    retry_delay: Duration,
}

impl ActorClient {
    /// A client for the API mounted at `base_url` (e.g. `http://node:8080` or `http://node:8080/internal`), on a new
    /// [`reqwest::Client`].
    pub fn new(base_url: impl AsRef<str>) -> Self {
        Self::with_client(base_url, reqwest::Client::new())
    }

    /// A client for the API mounted at `base_url` on `http`, whose configuration (authentication headers, timeouts,
    /// TLS) applies to every request.
    pub fn with_client(base_url: impl AsRef<str>, http: reqwest::Client) -> Self {
        Self {
            base: base_url.as_ref().trim_end_matches('/').to_owned(),
            http,
            send_attempts: 3,
            retry_delay: Duration::from_millis(200),
        }
    }

    /// Total attempts of a send on transport errors and `503`; default 3.
    ///
    /// # Panics
    ///
    /// When `attempts` is zero.
    pub fn send_attempts(mut self, attempts: u32) -> Self {
        assert!(attempts >= 1, "send_attempts must be at least 1");
        self.send_attempts = attempts;
        self
    }

    /// Pause before the second attempt of a send, doubled for each further one; default 200 ms.
    pub fn retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    // ---- instances ----

    /// Creates the instance at `address` with `state` (`PUT /actors/{path}`). Idempotent: an existing instance is left
    /// untouched and reported with `created == false`. Errors: `404` when no type matches, `422` when the state is
    /// refused (it is `null`, or the node hosts the type and cannot decode it).
    pub async fn create_actor(&self, address: &ActorAddress, state: Value) -> ClientResult<CreateActorResponse> {
        let body = encode(&CreateActorRequest { state })?;
        let answer = self.call(Method::PUT, self.url(&http_path(address)?, &[])?, Some(body)).await?;
        match answer.status {
            StatusCode::CREATED | StatusCode::OK => answer.decode(),
            _ => Err(answer.failure()),
        }
    }

    /// Creates `r` with `state`; see [`Self::create_actor`].
    pub async fn create<R: ActorRef, S: Serialize + ?Sized>(
        &self,
        r: &R,
        state: &S,
    ) -> ClientResult<CreateActorResponse> {
        let state = serde_json::to_value(state)
            .map_err(|e| ClientError::Invalid(format!("The initial state cannot be encoded: {e}")))?;
        self.create_actor(&address_of(r)?, state).await
    }

    /// Snapshot of the instance at `address` (`GET /actors/{path}`), or `None` when it does not exist.
    pub async fn get_actor(&self, address: &ActorAddress) -> ClientResult<Option<ActorView>> {
        let answer = self.call(Method::GET, self.url(&http_path(address)?, &[])?, None).await?;
        match answer.status {
            StatusCode::OK => answer.decode().map(Some),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_SUCH_INSTANCE),
            _ => Err(answer.failure()),
        }
    }

    /// Snapshot of `r`; see [`Self::get_actor`].
    pub async fn get<R: ActorRef>(&self, r: &R) -> ClientResult<Option<ActorView>> {
        self.get_actor(&address_of(r)?).await
    }

    /// The state of `r` decoded as `S`, or `None` when the instance does not exist.
    pub async fn state<R: ActorRef, S: DeserializeOwned>(&self, r: &R) -> ClientResult<Option<S>> {
        let Some(view) = self.get(r).await? else {
            return Ok(None);
        };
        serde_json::from_value(view.state)
            .map(Some)
            .map_err(|e| ClientError::Body(format!("The state of {} cannot be decoded: {e}", view.address)))
    }

    /// Admin delete (`DELETE /actors/{path}`); purges the mailbox. False when the instance did not exist.
    pub async fn delete_actor(&self, address: &ActorAddress) -> ClientResult<bool> {
        let answer = self.call(Method::DELETE, self.url(&http_path(address)?, &[])?, None).await?;
        match answer.status {
            StatusCode::NO_CONTENT => Ok(true),
            StatusCode::NOT_FOUND => answer.absent::<()>(problem_types::NO_SUCH_INSTANCE).map(|_| false),
            _ => Err(answer.failure()),
        }
    }

    /// Deletes `r`; see [`Self::delete_actor`].
    pub async fn delete<R: ActorRef>(&self, r: &R) -> ClientResult<bool> {
        self.delete_actor(&address_of(r)?).await
    }

    /// Instance URIs starting with `prefix` (`GET /actors?prefix=`), one page at a time; pass [`PageView::next`] as
    /// `cursor` for the following page.
    pub async fn list_actors(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> ClientResult<PageView<ActorAddress>> {
        let query = listing(prefix, None, cursor, limit);
        self.call(Method::GET, self.url("/actors", &query)?, None).await?.ok()
    }

    // ---- messages ----

    /// Sends `msg` to `to`, due now; the compiler checks that `to` accepts `msg`. See [`Self::send_message`].
    pub async fn send<R: ActorRef>(&self, to: &R, msg: impl Into<R::Message>) -> ClientResult<SendReceipt> {
        self.send_with(to, msg, SendOptions::default()).await
    }

    /// Sends `msg` to `to` with `options` (delivery time, id, correlation id, headers, retry and retention policies).
    /// See [`Self::send_message`].
    pub async fn send_with<R: ActorRef>(
        &self,
        to: &R,
        msg: impl Into<R::Message>,
        options: SendOptions,
    ) -> ClientResult<SendReceipt> {
        let encoded = message::encode(&msg.into()).map_err(|e| ClientError::Invalid(e.to_string()))?;
        self.send_message(&address_of(to)?, &encoded.message_type, encoded.payload, options).await
    }

    /// Sends a message given as type name and JSON payload (`POST /actors/{path}/messages`), for callers without the
    /// message types. Retried with the same id on transport errors and `503` (see the crate documentation); the
    /// policies of `options` are ignored when the id was already accepted. Errors: `404` when the instance does not
    /// exist, `422` for an unknown type name or (on a node hosting the type) an undecodable payload,
    /// `409` when the id was accepted for another message, `400` for an invalid envelope.
    pub async fn send_message(
        &self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        options: SendOptions,
    ) -> ClientResult<SendReceipt> {
        let request = SendRequest {
            id: Some(options.id.unwrap_or_else(MessageId::random)),
            correlation_id: options.correlation_id,
            headers: options.headers,
            retry: options.retry,
            retention: options.retention,
            ..SendRequest::new(message_type, payload)
        }
        .with_delivery(options.delivery);
        let body = encode(&request)?;
        let url = self.url(&send_path(to)?, &[])?;
        let mut wait = self.retry_delay;
        for attempt in 1..=self.send_attempts {
            let last = attempt == self.send_attempts;
            match self.call(Method::POST, url.clone(), Some(body.clone())).await {
                Err(ClientError::Http(e)) if !last && !e.is_builder() => {}
                Err(e) => return Err(e),
                Ok(answer) => match answer.status {
                    StatusCode::ACCEPTED => return receipt(&answer, false),
                    StatusCode::OK => return receipt(&answer, true),
                    StatusCode::SERVICE_UNAVAILABLE if !last => {}
                    _ => return Err(answer.failure()),
                },
            }
            tokio::time::sleep(wait).await;
            wait = wait.saturating_mul(2);
        }
        unreachable!("the last attempt returns")
    }

    /// The mailbox of `address` in delivery order (`GET /actors/{path}/mailbox`), each message marked running or
    /// pending, or `None` when the instance does not exist. Pages are not a snapshot: a message retried between two
    /// pages can appear on both; deduplicate by id.
    pub async fn mailbox(
        &self,
        address: &ActorAddress,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> ClientResult<Option<PageView<MailboxMessageView>>> {
        let url = self.url(&mailbox_path(address)?, &listing("", None, cursor, limit))?;
        let answer = self.call(Method::GET, url, None).await?;
        match answer.status {
            StatusCode::OK => answer.decode().map(Some),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_SUCH_INSTANCE),
            _ => Err(answer.failure()),
        }
    }

    /// Pauses the mailbox of `address` (`POST /actors/{path}/mailbox/pause`). False when the instance does not exist.
    pub async fn pause_mailbox(&self, address: &ActorAddress) -> ClientResult<bool> {
        self.mailbox_action(address, "pause").await
    }

    /// Resumes the mailbox of `address` (`POST /actors/{path}/mailbox/resume`). False when the instance does not exist.
    pub async fn resume_mailbox(&self, address: &ActorAddress) -> ClientResult<bool> {
        self.mailbox_action(address, "resume").await
    }

    async fn mailbox_action(&self, address: &ActorAddress, action: &str) -> ClientResult<bool> {
        let url = self.url(&format!("{}/mailbox/{action}", http_path(address)?), &[])?;
        let answer = self.call(Method::POST, url, None).await?;
        match answer.status {
            StatusCode::NO_CONTENT => Ok(true),
            StatusCode::NOT_FOUND => answer.absent::<()>(problem_types::NO_SUCH_INSTANCE).map(|_| false),
            _ => Err(answer.failure()),
        }
    }

    /// Gives outcome `purged` to every pending message of `address` except one being handled
    /// (`POST /actors/{path}/mailbox/purge`); returns how many, or `None` when the instance does not exist.
    pub async fn purge_mailbox(&self, address: &ActorAddress) -> ClientResult<Option<u64>> {
        let url = self.url(&format!("{}/mailbox/purge", http_path(address)?), &[])?;
        let answer = self.call(Method::POST, url, None).await?;
        match answer.status {
            StatusCode::OK => Ok(Some(answer.decode::<PurgeResponse>()?.purged)),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_SUCH_INSTANCE),
            _ => Err(answer.failure()),
        }
    }

    /// The pending messages of every instance whose URI starts with `prefix` (`GET /messages`): instance by instance in
    /// URI order, each mailbox in delivery order. Pages are not a snapshot; deduplicate by id.
    pub async fn list_pending(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> ClientResult<PageView<MessageView>> {
        let query = listing(prefix, None, cursor, limit);
        self.call(Method::GET, self.url("/messages", &query)?, None).await?.ok()
    }

    /// Status of message `id` (`GET /messages/{id}`), or `None` when unknown or past retention.
    pub async fn status(&self, id: &MessageId) -> ClientResult<Option<MessageStatusView>> {
        let answer = self.call(Method::GET, self.url(&message_path(id)?, &[])?, None).await?;
        match answer.status {
            StatusCode::OK => answer.decode().map(Some),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_SUCH_MESSAGE),
            _ => Err(answer.failure()),
        }
    }

    /// The messages sent by the turn that handled `id`, one level down (`GET /messages/{id}/caused`): pending ones and
    /// retained outcomes, in message-id order. `None` when `id` is unknown or past retention. Pages are not a snapshot.
    pub async fn caused_by(
        &self,
        id: &MessageId,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> ClientResult<Option<PageView<MessageStatusView>>> {
        let url = self.url(&format!("{}/caused", message_path(id)?), &listing("", None, cursor, limit))?;
        let answer = self.call(Method::GET, url, None).await?;
        match answer.status {
            StatusCode::OK => answer.decode().map(Some),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_SUCH_MESSAGE),
            _ => Err(answer.failure()),
        }
    }

    /// Admin cancel of a pending message (`POST /messages/{id}/cancel`).
    pub async fn cancel(&self, id: &MessageId) -> ClientResult<CancelOutcome> {
        let url = self.url(&format!("{}/cancel", message_path(id)?), &[])?;
        let answer = self.call(Method::POST, url, None).await?;
        match answer.status {
            StatusCode::NO_CONTENT => Ok(CancelOutcome::Cancelled),
            StatusCode::NOT_FOUND => {
                answer.absent::<()>(problem_types::NO_SUCH_MESSAGE).map(|_| CancelOutcome::NotFound)
            }
            StatusCode::CONFLICT => match answer.problem().problem_type.as_str() {
                problem_types::MESSAGE_RUNNING => Ok(CancelOutcome::Running),
                problem_types::MESSAGE_FINISHED => Ok(CancelOutcome::AlreadyFinished),
                _ => Err(answer.failure()),
            },
            _ => Err(answer.failure()),
        }
    }

    /// Sends a failed or cancelled message again as a new message with a new id (`POST /messages/{id}/retry`), or `None`
    /// when `id` is unknown or past retention. Other refusals are [`ClientError::Api`]: `404 no-such-instance` when the
    /// target is gone, `409 message-not-retryable` for a pending, running, done or purged message.
    pub async fn retry(&self, id: &MessageId) -> ClientResult<Option<SendReceipt>> {
        let url = self.url(&format!("{}/retry", message_path(id)?), &[])?;
        let answer = self.call(Method::POST, url, None).await?;
        match answer.status {
            StatusCode::ACCEPTED => Ok(Some(receipt(&answer, false)?)),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_SUCH_MESSAGE),
            _ => Err(answer.failure()),
        }
    }

    /// Makes a pending message due now, at a new place in delivery order (`POST /messages/{id}/deliver-now`), or `None`
    /// when `id` is unknown or past retention. A running or finished message is `409` as [`ClientError::Api`].
    pub async fn deliver_now(&self, id: &MessageId) -> ClientResult<Option<MessageView>> {
        let url = self.url(&format!("{}/deliver-now", message_path(id)?), &[])?;
        let answer = self.call(Method::POST, url, None).await?;
        match answer.status {
            StatusCode::OK => answer.decode().map(Some),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_SUCH_MESSAGE),
            _ => Err(answer.failure()),
        }
    }

    /// Cancels messages by ids or filter, at most 1000 per call (`POST /messages/bulk-cancel`).
    pub async fn bulk_cancel(&self, request: &BulkRequest) -> ClientResult<BulkResponse> {
        let url = self.url("/messages/bulk-cancel", &[])?;
        self.call(Method::POST, url, Some(encode(request)?)).await?.ok()
    }

    /// Retries messages by ids or filter, at most 1000 per call (`POST /messages/bulk-retry`).
    pub async fn bulk_retry(&self, request: &BulkRequest) -> ClientResult<BulkResponse> {
        let url = self.url("/messages/bulk-retry", &[])?;
        self.call(Method::POST, url, Some(encode(request)?)).await?.ok()
    }

    /// Finished messages with envelope and error (`GET /outcomes`), of one `status` or all, optionally limited to a
    /// URI `prefix` and to those that finished at or after `since`.
    pub async fn outcomes(
        &self,
        status: Option<OutcomeStatus>,
        prefix: &str,
        since: Option<Timestamp>,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> ClientResult<PageView<OutcomeView>> {
        let mut query = listing(prefix, since, cursor, limit);
        if let Some(status) = status {
            query.push(("status", status.wire_name().to_owned()));
        }
        self.call(Method::GET, self.url("/outcomes", &query)?, None).await?.ok()
    }

    /// The number of actors and pending messages under the URI `prefix` (empty: all), and the delivery time of the
    /// oldest due message (`GET /stats`).
    pub async fn stats(&self, prefix: &str) -> ClientResult<StatsView> {
        let query = listing(prefix, None, None, None);
        self.call(Method::GET, self.url("/stats", &query)?, None).await?.ok()
    }

    // ---- nodes and turns ----

    /// Every node with its status and the number of its running turns (`GET /nodes`).
    pub async fn nodes(&self) -> ClientResult<Vec<NodeView>> {
        let list: NodeListView = self.call(Method::GET, self.url("/nodes", &[])?, None).await?.ok()?;
        Ok(list.items)
    }

    /// One node with its status and running turns (`GET /nodes/{node}`), or `None`.
    pub async fn node(&self, id: &NodeId) -> ClientResult<Option<NodeDetailView>> {
        let url = self.url(&format!("/nodes/{}", percent::encode(id.as_str())), &[])?;
        let answer = self.call(Method::GET, url, None).await?;
        match answer.status {
            StatusCode::OK => answer.decode().map(Some),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_SUCH_NODE),
            _ => Err(answer.failure()),
        }
    }

    /// The turns running now, optionally of one `node` and under a URI `prefix` (`GET /turns`).
    pub async fn turns(
        &self,
        node: Option<&NodeId>,
        prefix: &str,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> ClientResult<PageView<RunningTurnView>> {
        let mut query = listing(prefix, None, cursor, limit);
        if let Some(node) = node {
            query.push(("node", node.to_string()));
        }
        self.call(Method::GET, self.url("/turns", &query)?, None).await?.ok()
    }

    /// The turn running now on `address` (`GET /actors/{path}/turn`), or `None`.
    pub async fn turn(&self, address: &ActorAddress) -> ClientResult<Option<RunningTurnView>> {
        let answer = self.call(Method::GET, self.url(&turn_path(address)?, &[])?, None).await?;
        match answer.status {
            StatusCode::OK => answer.decode().map(Some),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_RUNNING_TURN),
            _ => Err(answer.failure()),
        }
    }

    /// Asks the owning node to cancel the running turn of `address` (`POST /actors/{path}/turn/cancel`); it stops within
    /// one heartbeat. False when no turn is running.
    pub async fn cancel_turn(&self, address: &ActorAddress) -> ClientResult<bool> {
        let url = self.url(&format!("{}/cancel", turn_path(address)?), &[])?;
        let answer = self.call(Method::POST, url, None).await?;
        match answer.status {
            StatusCode::ACCEPTED => Ok(true),
            StatusCode::NOT_FOUND => answer.absent::<()>(problem_types::NO_RUNNING_TURN).map(|_| false),
            _ => Err(answer.failure()),
        }
    }

    // ---- types and health ----

    /// All registered types with their JSON Schemas (`GET /types`).
    pub async fn types(&self) -> ClientResult<Vec<TypeView>> {
        let list: TypeListView = self.call(Method::GET, self.url("/types", &[])?, None).await?.ok()?;
        Ok(list.items)
    }

    /// One registered type with its JSON Schemas (`GET /types/{name}`), or `None`.
    pub async fn get_type(&self, name: &TypeName) -> ClientResult<Option<TypeView>> {
        let answer = self.call(Method::GET, self.url(&format!("/types/{name}"), &[])?, None).await?;
        match answer.status {
            StatusCode::OK => answer.decode().map(Some),
            StatusCode::NOT_FOUND => answer.absent(problem_types::NO_SUCH_TYPE),
            _ => Err(answer.failure()),
        }
    }

    /// Removes the registration of the type `name` (`DELETE /types/{name}`): [`RemoveTypeResult::Removed`] (`204`),
    /// [`RemoveTypeResult::Absent`] (`404`) or [`RemoveTypeResult::InUse`] while actors of the type exist (`409`). A
    /// node that still hosts the type registers it again at its next start.
    pub async fn remove_type(&self, name: &TypeName) -> ClientResult<RemoveTypeResult> {
        let answer = self.call(Method::DELETE, self.url(&format!("/types/{name}"), &[])?, None).await?;
        match answer.status {
            StatusCode::NO_CONTENT => Ok(RemoveTypeResult::Removed),
            StatusCode::NOT_FOUND => answer.absent::<()>(problem_types::NO_SUCH_TYPE).map(|_| RemoveTypeResult::Absent),
            StatusCode::CONFLICT if answer.problem().problem_type == problem_types::TYPE_IN_USE => {
                Ok(RemoveTypeResult::InUse)
            }
            _ => Err(answer.failure()),
        }
    }

    /// True when the node is up and its store reachable (`GET /health` answers `200`); false on `503`.
    pub async fn health(&self) -> ClientResult<bool> {
        let answer = self.call(Method::GET, self.url("/health", &[])?, None).await?;
        match answer.status {
            StatusCode::OK => Ok(true),
            StatusCode::SERVICE_UNAVAILABLE => Ok(false),
            _ => Err(answer.failure()),
        }
    }

    // ---- plumbing ----

    fn url(&self, path: &str, query: &[(&str, String)]) -> ClientResult<Url> {
        let mut url = Url::parse(&format!("{}{path}", self.base))
            .map_err(|e| ClientError::Invalid(format!("Invalid URL {}{path}: {e}", self.base)))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url)
    }

    async fn call(&self, method: Method, url: Url, body: Option<String>) -> ClientResult<Answer> {
        let mut request = self.http.request(method, url);
        if let Some(body) = body {
            request = request.header(reqwest::header::CONTENT_TYPE, "application/json").body(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        Ok(Answer { status, text })
    }
}

fn receipt(answer: &Answer, duplicate: bool) -> ClientResult<SendReceipt> {
    let response: SendResponse = answer.decode()?;
    Ok(SendReceipt { id: response.id, deliver_at: response.deliver_at, duplicate })
}

fn encode<T: Serialize>(value: &T) -> ClientResult<String> {
    serde_json::to_string(value).map_err(|e| ClientError::Invalid(format!("The request cannot be encoded: {e}")))
}

fn address_of<R: ActorRef>(r: &R) -> ClientResult<ActorAddress> {
    r.address().map_err(|e| ClientError::Invalid(e.to_string()))
}

/// The query of a listing.
fn listing(
    prefix: &str,
    since: Option<Timestamp>,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if !prefix.is_empty() {
        query.push(("prefix", prefix.to_owned()));
    }
    if let Some(since) = since {
        query.push(("since", format_instant(since)));
    }
    if let Some(cursor) = cursor {
        query.push(("cursor", cursor.to_owned()));
    }
    if let Some(limit) = limit {
        query.push(("limit", limit.to_string()));
    }
    query
}

/// Refuses a path segment that URL parsing would remove (`.`, `..`), also escaped.
fn check_segment(segment: &str, what: &dyn std::fmt::Display) -> ClientResult<()> {
    if matches!(segment, "." | "..") {
        return Err(ClientError::Invalid(format!(
            "{what} has the segment '{segment}', which cannot be addressed by URL: URL parsing removes dot segments"
        )));
    }
    Ok(())
}

fn http_path(address: &ActorAddress) -> ClientResult<String> {
    Ok(format!("/actors/{address}"))
}

fn mailbox_path(address: &ActorAddress) -> ClientResult<String> {
    Ok(format!("/actors/{address}/mailbox"))
}

fn send_path(address: &ActorAddress) -> ClientResult<String> {
    Ok(format!("/actors/{address}/messages"))
}

fn turn_path(address: &ActorAddress) -> ClientResult<String> {
    Ok(format!("/actors/{address}/turn"))
}

fn message_path(id: &MessageId) -> ClientResult<String> {
    check_segment(id.as_str(), &format!("Message id {id}"))?;
    Ok(format!("/messages/{}", percent::encode(id.as_str())))
}
