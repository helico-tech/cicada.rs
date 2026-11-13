//! `GET /changes` (ADR 0035): the change hints as Server-Sent Events on both routers, their frames, query, headers,
//! lag, dropped bodies and lifetimes, on virtual time; and the guide's consumer, one serialized worker that lists after
//! a resync and rereads changed addresses, against a real stream.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use cicada::api::problem_types as pt;
use cicada::prelude::*;
use cicada_axum::{actor_api, actor_api_read};
use common::*;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::Value;
use tower::ServiceExt;

// ---- reading a stream ----

/// An open `GET /changes` body, read frame by frame.
struct Events {
    body: BoxStream<'static, Result<Bytes, axum::Error>>,
    buffer: String,
}

/// One frame: a comment, or an event with its name and data.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Frame {
    Comment(String),
    Event { name: String, data: String },
}

impl Events {
    /// The next whole frame; `None` at the end of the body.
    async fn frame(&mut self) -> Option<Frame> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let raw: String = self.buffer.drain(..end + 2).collect();
                return Some(parse(raw.trim_end_matches('\n')));
            }
            let chunk = self.body.next().await?.expect("the body has no error");
            self.buffer.push_str(std::str::from_utf8(&chunk).expect("UTF-8"));
        }
    }

    /// The next event, skipping comments, within a minute of virtual time: `None` for nothing, `Some(None)` for the
    /// end of the body.
    async fn event(&mut self) -> Option<Option<(String, String)>> {
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                match self.frame().await {
                    Some(Frame::Event { name, data }) => return Some((name, data)),
                    Some(Frame::Comment(_)) => {}
                    None => return None,
                }
            }
        })
        .await
        .ok()
    }

    /// The event that is ready now, if any, skipping comments.
    async fn ready(&mut self) -> Option<Option<(String, String)>> {
        tokio::time::timeout(Duration::from_millis(1), async {
            loop {
                match self.frame().await {
                    Some(Frame::Event { name, data }) => return Some((name, data)),
                    Some(Frame::Comment(_)) => {}
                    None => return None,
                }
            }
        })
        .await
        .ok()
    }
}

fn parse(raw: &str) -> Frame {
    if let Some(comment) = raw.strip_prefix(':') {
        return Frame::Comment(comment.trim().to_owned());
    }
    let (mut name, mut data) = (String::new(), String::new());
    for line in raw.lines() {
        match line.split_once(": ") {
            Some(("event", value)) => name = value.to_owned(),
            Some(("data", value)) => data = value.to_owned(),
            _ => panic!("an unexpected line in {raw:?}: {line:?}"),
        }
    }
    Frame::Event { name, data }
}

/// Sends `GET uri` with `headers` and returns the status, the headers and the open body.
async fn open_with(app: &Router, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap, Events) {
    let mut builder = Request::builder().uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app.clone().oneshot(builder.body(Body::empty()).unwrap()).await.unwrap();
    let (parts, body) = response.into_parts();
    (parts.status, parts.headers, Events { body: body.into_data_stream().boxed(), buffer: String::new() })
}

async fn open(app: &Router, uri: &str) -> Events {
    let (status, headers, events) = open_with(app, uri, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "text/event-stream");
    events
}

fn resync() -> Option<Option<(String, String)>> {
    Some(Some(("resync".to_owned(), "{}".to_owned())))
}

fn changed(address: &str) -> Option<Option<(String, String)>> {
    Some(Some(("changed".to_owned(), format!(r#"{{"address":"{address}"}}"#))))
}

fn counter(id: &str) -> Counter {
    Counter { id: id.to_owned() }
}

fn address(id: &str) -> ActorAddress {
    counter(id).address().unwrap()
}

// ---- the route ----

/// Scenarios: Both routers and host layers; First application event; A changed address.
#[tokio::test(start_paused = true)]
async fn both_routers_stream_a_first_resync_then_each_changed_address_with_sse_headers_and_no_id() {
    let system = node(memory_store()).await;
    for (n, app) in [actor_api(system.clone()), actor_api_read(system.clone())].into_iter().enumerate() {
        let (status, headers, mut events) = open_with(&app, "/changes", &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "text/event-stream");
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
        assert_eq!(headers["x-accel-buffering"], "no");
        assert_eq!(events.frame().await, Some(Frame::Event { name: "resync".into(), data: "{}".into() }));
        let id = format!("c{n}");
        system.create(&counter(&id), &CounterState::default()).await.unwrap();
        assert_eq!(events.event().await, changed(&format!("counter/{id}")));
        // `parse` refuses any line but `event` and `data`, so no `id` or `retry` came; and no comment before 15 s.
        system.create(&counter(&format!("{id}-2")), &CounterState::default()).await.unwrap();
        assert_eq!(
            events.frame().await,
            Some(Frame::Event { name: "changed".into(), data: format!(r#"{{"address":"counter/{id}-2"}}"#) })
        );
        let reply = get(&app, &format!("/actors/counter/{id}")).await;
        assert_eq!(reply.status, StatusCode::OK, "the hint is followed by a read");
    }
}

/// Scenario: Both routers and host layers.
#[tokio::test(start_paused = true)]
async fn the_hosts_layers_wrap_the_stream() {
    let system = node(memory_store()).await;
    let app = actor_api_read(system).layer(middleware::from_fn(|req: Request, next: Next| async move {
        if req.headers().get(header::AUTHORIZATION).is_some_and(|v| v == "Bearer t") {
            next.run(req).await
        } else {
            StatusCode::UNAUTHORIZED.into_response()
        }
    }));
    let (status, _, mut events) = open_with(&app, "/changes", &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(events.frame().await, None);
    let (status, _, mut events) = open_with(&app, "/changes", &[("authorization", "Bearer t")]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(events.event().await, resync());
}

/// Scenario: A changed address (canonical encoding) and Encoding and disposal (the address is the URL suffix as it is).
#[tokio::test(start_paused = true)]
async fn a_changed_address_is_canonical_and_is_the_suffix_of_its_actor_url() {
    let system = node(memory_store()).await;
    let app = actor_api(system.clone());
    let mut events = open(&app, "/changes").await;
    assert_eq!(events.event().await, resync());
    for (id, canonical) in [("jan@example.com", "counter/jan%40example.com"), ("a/b%c", "counter/a%2Fb%25c")] {
        system.create(&counter(id), &CounterState::default()).await.unwrap();
        assert_eq!(events.event().await, changed(canonical));
        let actor = get_json(&app, &format!("/actors/{canonical}")).await;
        assert_eq!(actor["address"], canonical);
    }
}

/// Scenario: Literal first prefix.
#[tokio::test(start_paused = true)]
async fn the_first_prefix_is_matched_literally_once_decoded_and_other_keys_add_nothing() {
    let system = node(memory_store()).await;
    let app = actor_api(system.clone());
    let queries = [
        ("/changes", "ping/x"),
        ("/changes?prefix=", "ping/x"),
        ("/changes?prefix=counter", "counter/jan%40example.com"),
        ("/changes?prefix=counter%2F", "counter/jan%40example.com"),
        ("/changes?prefix=counter%2Fjan%2540", "counter/jan%40example.com"),
        ("/changes?prefix=counter%2F&prefix=ping%2F", "counter/jan%40example.com"),
        ("/changes?limit=0&cursor=zzz&prefix=counter%2F&other=1", "counter/jan%40example.com"),
    ];
    let mut streams = Vec::new();
    for (uri, _) in queries {
        let mut events = open(&app, uri).await;
        assert_eq!(events.event().await, resync(), "{uri}");
        streams.push(events);
    }
    system.create(&Ping { name: "x".into() }, &NoState {}).await.unwrap();
    system.create(&counter("jan@example.com"), &CounterState::default()).await.unwrap();
    for ((uri, first), events) in queries.iter().zip(&mut streams) {
        assert_eq!(events.event().await, changed(first), "{uri}");
    }
    // `counter/jan%40` is the literal prefix: `counter/jan@` and the once-encoded `counter/jan%40` on the wire are not.
    for uri in ["/changes?prefix=counter%2Fjan%40", "/changes?prefix=counter/jan%40"] {
        let mut events = open(&app, uri).await;
        assert_eq!(events.event().await, resync());
        system.create(&counter(&format!("jan@{}", unique())), &CounterState::default()).await.unwrap();
        assert_eq!(events.event().await, None, "{uri}");
    }
}

/// Scenario: Reconnect with Last-Event-ID.
#[tokio::test(start_paused = true)]
async fn last_event_id_is_ignored_and_a_new_request_starts_with_a_resync() {
    let system = node(memory_store()).await;
    let app = actor_api(system.clone());
    system.create(&counter("a"), &CounterState::default()).await.unwrap();
    let (status, _, mut events) = open_with(&app, "/changes", &[("last-event-id", "42")]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(events.event().await, resync(), "no replay of the create before it");
    system.create(&counter("b"), &CounterState::default()).await.unwrap();
    assert_eq!(events.event().await, changed("counter/b"));
}

/// Scenario: Slow body reader.
#[tokio::test(start_paused = true)]
async fn a_body_that_is_not_read_gets_the_cores_resync_and_no_queue_of_its_own() {
    let system = node(memory_store()).await;
    let app = actor_api(system.clone());
    let mut events = open(&app, "/changes").await;
    assert_eq!(events.event().await, resync());
    for i in 0..1100 {
        system.create(&counter(&format!("n{i:04}")), &CounterState::default()).await.unwrap();
    }
    // The lagged backlog is dropped: the next event is the core's resync, then the hints after it.
    assert_eq!(events.event().await, resync(), "the lagged stream resyncs");
    system.create(&counter("z"), &CounterState::default()).await.unwrap();
    assert_eq!(events.event().await, changed("counter/z"), "nothing of the 1100 creates was queued for the body");
}

/// Scenario: Client disconnect.
#[tokio::test(start_paused = true)]
async fn a_dropped_body_leaves_other_streams_and_requests_working() {
    let system = node(memory_store()).await;
    let app = actor_api(system.clone());
    let mut kept = open(&app, "/changes").await;
    let mut dropped = open(&app, "/changes").await;
    assert_eq!(kept.event().await, resync());
    assert_eq!(dropped.event().await, resync());
    system.create(&counter("a"), &CounterState::default()).await.unwrap();
    drop(dropped); // with a hint unread
    let idle = open(&app, "/changes").await;
    drop(idle); // before its first event
    system.create(&counter("b"), &CounterState::default()).await.unwrap();
    assert_eq!(kept.event().await, changed("counter/a"));
    assert_eq!(kept.event().await, changed("counter/b"));
    assert_eq!(items(&get_json(&app, "/actors").await).len(), 2);
}

/// Scenarios: Store close; HTTP changes after close.
#[tokio::test(start_paused = true)]
async fn closing_the_store_ends_open_streams_without_an_event_and_a_later_request_gets_an_empty_200() {
    let store = memory_store();
    let system = node(store.clone()).await;
    let app = actor_api(system.clone());
    let mut events = open(&app, "/changes").await;
    assert_eq!(events.event().await, resync());
    system.create(&counter("a"), &CounterState::default()).await.unwrap(); // a backlog the close discards
    system.stop().await;
    store.close().await;
    assert_eq!(events.frame().await, None, "ended, with no final event");
    let (status, headers, mut after) = open_with(&app, "/changes", &[]).await;
    assert_eq!((status, &headers[header::CONTENT_TYPE]), (StatusCode::OK, &"text/event-stream".parse().unwrap()));
    assert_eq!(after.frame().await, None, "an empty body that ends at once");
    assert_problem(&get(&app, "/actors").await, 503, pt::STORE_UNAVAILABLE);
}

/// Scenario: Worker stop.
#[tokio::test(start_paused = true)]
async fn stopping_the_node_leaves_its_streams_open() {
    let store = memory_store();
    let system = node(store.clone()).await;
    let app = actor_api(system.clone());
    let mut events = open(&app, "/changes").await;
    assert_eq!(events.event().await, resync());
    system.stop().await;
    let other = node(store).await;
    other.create(&counter("a"), &CounterState::default()).await.unwrap();
    assert_eq!(events.event().await, changed("counter/a"));
}

// ---- the guide's consumer ----

/// The consumer of the HTTP guide, in Rust: one serialized worker, a bounded set of dirty addresses and a generation
/// that a resync (or an overflow of the set) moves on, so that an older, incomplete listing is never published.
struct Consumer {
    app: Router,
    prefix: String,
    page: usize,
    max_dirty: usize,
    events: Events,
    view: BTreeMap<String, Value>,
    dirty: BTreeSet<String>,
    generation: u64,
    needs_listing: bool,
    /// Listings published, listings dropped for a newer generation, and reads that failed.
    published: usize,
    abandoned: usize,
    failures: usize,
    most_dirty: usize,
}

const RETRY: Duration = Duration::from_secs(1);

impl Consumer {
    /// Opens the stream and waits for its first resync, as the page waits before its first full read.
    async fn open(app: &Router, prefix: &str, page: usize, max_dirty: usize) -> Self {
        let events = open(app, &format!("/changes?prefix={}", enc(prefix))).await;
        let mut consumer = Self {
            app: app.clone(),
            prefix: prefix.to_owned(),
            page,
            max_dirty,
            events,
            view: BTreeMap::new(),
            dirty: BTreeSet::new(),
            generation: 0,
            needs_listing: false,
            published: 0,
            abandoned: 0,
            failures: 0,
            most_dirty: 0,
        };
        let first = consumer.events.event().await.expect("an event").expect("open");
        consumer.handle(first);
        assert!(consumer.needs_listing, "the first event is a resync");
        consumer
    }

    fn handle(&mut self, (name, data): (String, String)) {
        match name.as_str() {
            "resync" => self.resync(),
            "changed" => {
                let address = serde_json::from_str::<Value>(&data).unwrap()["address"].as_str().unwrap().to_owned();
                if !self.dirty.contains(&address) && self.dirty.len() == self.max_dirty {
                    self.resync(); // too many to keep: read everything again
                } else {
                    self.dirty.insert(address);
                    self.most_dirty = self.most_dirty.max(self.dirty.len());
                }
            }
            _ => {} // unknown events are ignored
        }
    }

    fn resync(&mut self) {
        self.generation += 1;
        self.dirty.clear();
        self.needs_listing = true;
    }

    /// Handles the events that have arrived, as EventSource would while a read is in flight.
    async fn pump(&mut self) {
        while let Some(Some(event)) = self.events.ready().await {
            self.handle(event);
        }
    }

    /// Works until nothing is left to do; `between_pages` runs after each page of a listing is read.
    async fn settle(&mut self, mut between_pages: impl AsyncFnMut(usize)) {
        loop {
            self.pump().await;
            if self.needs_listing {
                self.needs_listing = false;
                self.list(&mut between_pages).await;
            } else if let Some(address) = self.dirty.pop_first() {
                let reply = get(&self.app, &format!("/actors/{address}")).await;
                match reply.status {
                    StatusCode::OK => {
                        self.view.insert(address, reply.json());
                    }
                    StatusCode::NOT_FOUND => {
                        self.view.remove(&address);
                    }
                    _ => {
                        self.failures += 1;
                        self.dirty.insert(address); // kept for the retry
                        tokio::time::sleep(RETRY).await;
                    }
                }
            } else {
                return;
            }
        }
    }

    /// Reads every page into a fresh map and publishes it only when it is complete and still current.
    async fn list(&mut self, between_pages: &mut impl AsyncFnMut(usize)) {
        let generation = self.generation;
        let mut fresh = BTreeMap::new();
        let mut cursor: Option<String> = None;
        for n in 0.. {
            let mut uri = format!("/actors/states?prefix={}&limit={}", enc(&self.prefix), self.page);
            if let Some(cursor) = &cursor {
                uri.push_str(&format!("&cursor={}", enc(cursor)));
            }
            let reply = get(&self.app, &uri).await;
            if reply.status != StatusCode::OK {
                self.failures += 1;
                self.needs_listing = true;
                tokio::time::sleep(RETRY).await;
                return;
            }
            let page = reply.json();
            for actor in items(&page) {
                fresh.insert(actor["address"].as_str().unwrap().to_owned(), actor.clone());
            }
            between_pages(n).await;
            self.pump().await;
            if self.generation != generation {
                self.abandoned += 1; // a newer resync: this listing may miss what it reports
                return;
            }
            match page["next"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        self.view = fresh;
        self.published += 1;
    }
}

/// What the store holds now under `prefix`, by address.
async fn truth(app: &Router, prefix: &str) -> BTreeMap<String, Value> {
    let page = get_json(app, &format!("/actors/states?prefix={}&limit=1000", enc(prefix))).await;
    assert!(page["next"].is_null());
    items(&page).iter().map(|a| (a["address"].as_str().unwrap().to_owned(), a.clone())).collect()
}

async fn counters(system: &ActorSystem, ids: &[&str]) {
    for id in ids {
        system.create(&counter(id), &CounterState::default()).await.unwrap();
    }
}

/// Scenario: Hints during paging (they are kept and reread after the complete listing).
#[tokio::test(start_paused = true)]
async fn hints_during_paging_are_reread_after_the_listing_is_published() {
    let system = node(memory_store()).await;
    let app = actor_api(system.clone());
    counters(&system, &["a", "b", "c", "d", "e"]).await;
    let mut consumer = Consumer::open(&app, "counter/", 2, 1000).await;
    consumer
        .settle(async |page| {
            if page == 0 {
                // After a and b: one listed actor changes, one appears behind the cursor, one goes away.
                system.pause_mailbox(&address("a")).await.unwrap();
                counters(&system, &["aa"]).await;
                system.delete_actor(&address("e")).await.unwrap();
            }
        })
        .await;
    assert_eq!((consumer.published, consumer.abandoned), (1, 0));
    assert_eq!(consumer.view, truth(&app, "counter/").await);
    assert_eq!(consumer.view["counter/a"]["paused"], true);
    assert!(consumer.view.contains_key("counter/aa") && !consumer.view.contains_key("counter/e"));
}

/// Scenario: Hints during paging (a newer resync invalidates the older, incomplete listing).
#[tokio::test(start_paused = true)]
async fn a_resync_during_paging_drops_the_incomplete_listing() {
    let system = node(memory_store()).await;
    let app = actor_api(system.clone());
    counters(&system, &["a", "b", "c"]).await;
    let mut consumer = Consumer::open(&app, "counter/", 2, 1000).await;
    let ids: Vec<String> = (0..101).map(|i| format!("a-{i:03}")).collect();
    consumer
        .settle(async |page| {
            if page == 0 && not_spawned_yet(&system).await {
                // One turn creates 101 children: more than 100 addresses in one commit are one resync.
                system.send(&counter("a"), CounterMsg::Spawn { ids: ids.clone() }).await.unwrap();
                eventually(async || {
                    system.list_children(&address("a"), None, 1000).await.unwrap().unwrap().items.len() == 101
                })
                .await;
            }
        })
        .await;
    assert_eq!(consumer.abandoned, 1, "the first listing was dropped");
    assert_eq!(consumer.published, 1);
    assert_eq!(consumer.view, truth(&app, "counter/").await);
    assert_eq!(consumer.view.len(), 104);
}

/// Whether the children of the resync test are still to be created, so its hook acts once.
async fn not_spawned_yet(system: &ActorSystem) -> bool {
    system.get_actor(&address("a-000")).await.unwrap().is_none()
}

/// Scenario: Overflow or fetch failure (the set of dirty addresses is bounded).
#[tokio::test(start_paused = true)]
async fn more_dirty_addresses_than_the_bound_turn_into_a_new_listing() {
    let system = node(memory_store()).await;
    let app = actor_api(system.clone());
    counters(&system, &["a", "b", "c"]).await;
    let mut consumer = Consumer::open(&app, "counter/", 2, 3).await;
    consumer
        .settle(async |page| {
            if page == 0 && system.get_actor(&address("n4")).await.unwrap().is_none() {
                counters(&system, &["n1", "n2", "n3", "n4"]).await;
            }
        })
        .await;
    assert_eq!(consumer.most_dirty, 3, "never more than the bound");
    assert_eq!(consumer.abandoned, 1, "the fourth address asked for a new listing");
    assert_eq!(consumer.view, truth(&app, "counter/").await);
    assert_eq!(consumer.view.len(), 7);
}

/// Scenario: Overflow or fetch failure (a failed listing and a failed read are retried, and nothing is lost).
#[tokio::test(start_paused = true)]
async fn a_failed_listing_and_a_failed_read_are_retried_without_losing_the_address() {
    let system = node(memory_store()).await;
    let failing = Arc::new(AtomicUsize::new(0));
    let fail = failing.clone();
    let app = actor_api(system.clone()).layer(middleware::from_fn(move |req: Request, next: Next| {
        let fail = fail.clone();
        async move {
            let refuse = req.uri().path() != "/changes"
                && fail.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok();
            if refuse { StatusCode::SERVICE_UNAVAILABLE.into_response() } else { next.run(req).await }
        }
    }));
    counters(&system, &["a", "b"]).await;
    let mut consumer = Consumer::open(&app, "counter/", 2, 1000).await;
    failing.store(1, Ordering::SeqCst); // the first listing fails
    consumer.settle(async |_| {}).await;
    assert_eq!((consumer.failures, consumer.published), (1, 1));
    assert_eq!(consumer.view, truth(&app, "counter/").await);

    system.pause_mailbox(&address("b")).await.unwrap();
    failing.store(1, Ordering::SeqCst); // the reread of b fails once
    consumer.settle(async |_| {}).await;
    assert_eq!(consumer.failures, 2);
    assert_eq!(consumer.view["counter/b"]["paused"], true, "b was kept and read again");
    assert_eq!(consumer.view, truth(&app, "counter/").await);
}

// ---- the HTTP guide's standalone host ----

/// Scenario: Embedded axum host. The shutdown future of the HTTP guide stops the node and then closes the store, so an
/// open change stream ends and axum's graceful drain can finish; on real time and TCP, like a host.
#[tokio::test]
async fn the_guides_shutdown_stops_the_node_then_closes_the_store_and_so_ends_an_open_stream() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let store = memory_store();
    let system = node(store.clone()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let server = {
        let system = system.clone();
        tokio::spawn(async move {
            axum::serve(listener, actor_api(system.clone()))
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                    system.stop().await;
                    store.close().await;
                })
                .await
        })
    };
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client.write_all(b"GET /changes HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
    let mut received = Vec::new();
    let mut chunk = [0u8; 1024];
    while !String::from_utf8_lossy(&received).contains("event: resync") {
        let n = tokio::time::timeout(Duration::from_secs(2), client.read(&mut chunk)).await.unwrap().unwrap();
        assert!(n > 0, "the stream ended early");
        received.extend_from_slice(&chunk[..n]);
    }
    tx.send(()).unwrap();
    let served = tokio::time::timeout(Duration::from_secs(5), server).await.expect("the drain finishes");
    served.unwrap().expect("served");
    assert_eq!(system.state(), cicada::runtime::NodeState::Stopped);
    tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut received)).await.unwrap().unwrap();
    let text = String::from_utf8_lossy(&received);
    assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
    assert!(text.ends_with("0\r\n\r\n"), "the chunked body ended cleanly, with no final event: {text:?}");
    assert_eq!(text.matches("event: ").count(), 1, "{text:?}");
}
