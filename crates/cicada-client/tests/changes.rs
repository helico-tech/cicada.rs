//! The change reader against loopback HTTP, including the real actor API.

mod common;

use std::error::Error;
use std::fmt::{Debug, Display};
use std::future::{Future, pending};
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{Next, from_fn};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use cicada::ActorRef;
use cicada::api::{Problem, problem_types};
use cicada_axum::actor_api;
use cicada_client::{ActorClient, Change, ChangeStreamError, ClientError};
use cicada_test::TestActors;
use futures::{StreamExt, poll, stream};
use tokio::sync::Notify;
use tokio::time::{advance, sleep, timeout};

use common::{AccountActor, AccountState, account, serve};

const TOKEN: &str = "Bearer unique-test-token";
const PASSWORD: &str = "unique-url-password";

/// Give real sockets time to respond in a paused runtime while still bounding a hung test by the wall clock.
async fn within_wall_clock<T>(test: impl Future<Output = T>) -> T {
    let (done, wait) = std::sync::mpsc::channel();
    let watchdog = tokio::task::spawn_blocking(move || wait.recv_timeout(Duration::from_secs(8)));
    let result = tokio::select! {
        result = test => result,
        _ = watchdog => panic!("test did not finish within 8 wall-clock seconds"),
    };
    let _ = done.send(());
    result
}

fn assert_no_secrets(error: &(impl Display + Debug)) {
    for text in [error.to_string(), format!("{error:?}")] {
        assert!(!text.contains("unique-test-token"));
        assert!(!text.contains(PASSWORD));
    }
}

fn sse(body: Body) -> Response {
    ([(header::CONTENT_TYPE, "Text/Event-Stream; charset=utf-8")], body).into_response()
}

fn transport_source(error: &ChangeStreamError) -> &reqwest::Error {
    assert!(matches!(error, ChangeStreamError::Transport(_)));
    assert_no_secrets(error);
    let source = error.source().unwrap().downcast_ref::<reqwest::Error>().unwrap();
    assert!(source.url().is_none());
    assert_no_secrets(source);
    source
}

#[tokio::test(start_paused = true)]
async fn streams_scoped_store_hints_with_configured_headers_until_the_store_closes() {
    within_wall_clock(async {
        let actors = TestActors::builder().with(|node| node.host(AccountActor)).build().await.unwrap();
        let app = actor_api(actors.system().clone()).layer(from_fn(|request: Request, next: Next| async move {
            assert_eq!(request.headers().get(header::AUTHORIZATION).unwrap(), TOKEN);
            assert_eq!(request.headers().get("x-context").unwrap(), "test context");
            next.run(request).await
        }));
        let base = serve(app).await;
        let mut headers = header::HeaderMap::new();
        headers.insert(header::AUTHORIZATION, TOKEN.parse().unwrap());
        headers.insert("x-context", "test context".parse().unwrap());
        let client =
            ActorClient::with_client(base, reqwest::Client::builder().default_headers(headers).build().unwrap());

        let actor = account("a@b", 1);
        let address = actor.address().unwrap();
        let mut changes = client.changes(address.as_str()).await.unwrap();
        assert_eq!(changes.next().await.unwrap().unwrap(), Change::Resync);
        actors.create(&account("outside", 1), &AccountState::default()).await;
        actors.create(&actor, &AccountState::default()).await;
        assert_eq!(changes.next().await.unwrap().unwrap(), Change::Changed(address));
        actors.store().close().await;
        assert!(changes.next().await.is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn refuses_non_200_statuses_and_wrong_media_types_before_returning_a_stream() {
    within_wall_clock(async {
        let app = Router::new()
            .route("/changes", get(|| async { (StatusCode::UNAUTHORIZED, format!("{TOKEN} {PASSWORD}")) }))
            .route(
                "/missing/changes",
                get(|| async {
                    let problem = Problem {
                        problem_type: problem_types::NO_SUCH_INSTANCE.into(),
                        title: "No such instance".into(),
                        status: 404,
                        detail: Some("The requested resource is absent".into()),
                        instance: Some("/missing/changes".into()),
                    };
                    (StatusCode::NOT_FOUND, Json(problem))
                }),
            )
            .route("/html/changes", get(|| async { ([(header::CONTENT_TYPE, "text/html")], PASSWORD) }))
            .route("/no-type/changes", get(|| async { Body::empty() }));
        let base = serve(app).await;
        let denied = ActorClient::new(&base).changes("").await.err().expect("401 opening the stream");
        assert!(matches!(denied, ClientError::Api(ref problem) if problem.status == 401 && problem.detail.is_none()));
        assert_no_secrets(&denied);

        let missing = ActorClient::new(format!("{base}/missing")).changes("").await.err().expect("404 opening");
        assert_eq!(missing.status(), Some(404));
        let problem = missing.problem().unwrap();
        assert_eq!(problem.problem_type, problem_types::NO_SUCH_INSTANCE);
        assert_eq!(problem.detail.as_deref(), Some("The requested resource is absent"));
        assert_eq!(problem.instance.as_deref(), Some("/missing/changes"));

        for path in ["html", "no-type"] {
            let wrong_type = ActorClient::new(format!("{base}/{path}")).changes("").await.err().expect("invalid MIME");
            assert!(matches!(wrong_type, ClientError::Body(_)));
            assert_no_secrets(&wrong_type);
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn times_out_after_ten_seconds_without_response_headers() {
    within_wall_clock(async {
        let arrived = Arc::new(Notify::new());
        let seen = arrived.clone();
        let app = Router::new().route(
            "/changes",
            get(move || async move {
                seen.notify_one();
                pending::<Response>().await
            }),
        );
        let client = ActorClient::new(serve(app).await);
        let mut opening = pin!(client.changes(""));
        tokio::select! {
            _ = arrived.notified() => (),
            _ = &mut opening => panic!("opening completed before the server received the request"),
        }
        advance(Duration::from_secs(9)).await;
        assert!(poll!(&mut opening).is_pending());
        advance(Duration::from_secs(1)).await;
        let error = opening.await.err().expect("handshake timeout");
        assert!(matches!(error, ClientError::Body(ref message) if message == "no response headers within 10 s"));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_http_read_timeout_ends_the_stream_with_transport() {
    within_wall_clock(async {
        let app = Router::new().route(
            "/changes",
            get(|| async {
                let first = Ok::<_, std::io::Error>(Bytes::from_static(b"event: resync\ndata: {}\n\n"));
                sse(Body::from_stream(stream::iter([first]).chain(stream::pending())))
            }),
        );
        let http = reqwest::Client::builder().read_timeout(Duration::from_secs(2)).build().unwrap();
        let base = serve(app).await.replacen("http://", &format!("http://user:{PASSWORD}@"), 1);
        let client = ActorClient::with_client(base, http);
        let mut events = client.changes("").await.unwrap();
        assert_eq!(events.next().await.unwrap().unwrap(), Change::Resync);
        let mut next = pin!(events.next());
        assert!(poll!(&mut next).is_pending());
        advance(Duration::from_secs(3)).await;
        let error = next.await.unwrap().unwrap_err();
        assert!(transport_source(&error).is_timeout());
        assert!(events.next().await.is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_dropped_http_body_keeps_a_non_timeout_transport_source() {
    within_wall_clock(async {
        let dropped = Arc::new(Notify::new());
        let signal = dropped.clone();
        let app = Router::new().route(
            "/changes",
            get(move || async move {
                let first = Ok(Bytes::from_static(b"event: resync\ndata: {}\n\n"));
                let failure = stream::once(async move {
                    signal.notified().await;
                    Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset))
                });
                sse(Body::from_stream(stream::iter([first]).chain(failure)))
            }),
        );
        let base = serve(app).await.replacen("http://", &format!("http://user:{PASSWORD}@"), 1);
        let client = ActorClient::new(base);
        let mut events = client.changes("").await.unwrap();
        assert_eq!(events.next().await.unwrap().unwrap(), Change::Resync);
        dropped.notify_one();
        let error = events.next().await.unwrap().unwrap_err();
        assert!(!transport_source(&error).is_timeout());
        assert!(events.next().await.is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn credentials_stay_out_of_errors_and_client_debug() {
    within_wall_clock(async {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::AUTHORIZATION, TOKEN.parse().unwrap());
        let http = reqwest::Client::builder().default_headers(headers).build().unwrap();
        for (base, is_invalid_url) in [
            (format!("ftp://user:{PASSWORD}@localhost"), false),
            (format!("http://user:{PASSWORD}@localhost:bad-port"), true),
        ] {
            let client = ActorClient::with_client(base, http.clone());
            let debug = format!("{client:?}");
            assert!(!debug.contains(TOKEN));
            assert!(!debug.contains(PASSWORD));
            let error = client.changes("").await.err().expect("cannot send request");
            assert_eq!(matches!(error, ClientError::Invalid(_)), is_invalid_url);
            assert_no_secrets(&error);
        }

        let app = Router::new().route(
            "/changes",
            get(|| async {
                let body = format!("event: changed\ndata: {TOKEN} {PASSWORD}\n\n");
                sse(Body::from(body))
            }),
        );
        let base = serve(app).await;
        let with_credentials = base.replacen("http://", &format!("http://user:{PASSWORD}@"), 1);
        let debug =
            format!("{:?}", ActorClient::with_client(format!("{with_credentials}?auth={PASSWORD}"), http.clone()));
        assert!(debug.contains(&format!("base: {base:?}")));
        assert!(!debug.contains(TOKEN));
        assert!(!debug.contains(PASSWORD));
        let client = ActorClient::with_client(with_credentials, http);
        let mut events = client.changes("").await.unwrap();
        let error = events.next().await.unwrap().unwrap_err();
        assert!(matches!(error, ChangeStreamError::Malformed(_)));
        assert_no_secrets(&error);
        assert!(events.next().await.is_none());
    })
    .await;
}

#[tokio::test]
async fn a_one_second_client_timeout_does_not_cut_off_three_seconds_of_keep_alives() {
    timeout(Duration::from_secs(7), async {
        let app = Router::new()
            .route(
                "/changes",
                get(|| async {
                    let chunks = stream::iter(0..=15).then(|index| async move {
                        if index > 0 {
                            sleep(Duration::from_millis(200)).await;
                        }
                        let frame = if index == 15 { "event: resync\ndata: {}\n\n" } else { ": keep-alive\n\n" };
                        Ok::<_, std::io::Error>(Bytes::from_static(frame.as_bytes()))
                    });
                    sse(Body::from_stream(chunks))
                }),
            )
            .route(
                "/health",
                get(|| async {
                    sleep(Duration::from_secs(3)).await;
                    StatusCode::OK
                }),
            );
        let http = reqwest::Client::builder().timeout(Duration::from_secs(1)).build().unwrap();
        let client = ActorClient::with_client(serve(app).await, http);
        let mut events = client.changes("").await.unwrap();
        let (change, health) = tokio::join!(events.next(), client.health());
        assert_eq!(change.unwrap().unwrap(), Change::Resync);
        assert!(matches!(health, Err(ClientError::Http(error)) if error.is_timeout()));
        assert!(events.next().await.is_none());
    })
    .await
    .unwrap();
}
