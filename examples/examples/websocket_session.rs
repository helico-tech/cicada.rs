//! Example 20 as a running node: `cargo run -p cicada-examples --example websocket_session`.
//!
//! It opens a SQLite store in the temporary folder, creates a session under a fresh stream key and its producer,
//! serves the session route and a minimal browser client on loopback, and emits a few frames. Open the printed page;
//! reload it, or stop and reconnect, to see unacknowledged frames replayed. Ctrl-C stops it. The route has no
//! authentication: a real host checks the caller before the upgrade.

use std::time::Duration;

use axum::response::Html;
use axum::routing::get;
use cicada::store::{MessageId, SqliteOptions};
use cicada::{ActorSystem, Store};
use cicada_examples::websocket_session::{
    Producer, ProducerActor, ProducerMsg, ProducerState, Session, SessionActor, SessionState, adapter,
};
use tokio_util::sync::CancellationToken;

const CLIENT: &str = include_str!("websocket_session.html");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::temp_dir().join("cicada-websocket-session.db");
    let store = Store::sqlite(&path, SqliteOptions::default()).await?;
    let system = ActorSystem::builder(store.clone()).host(SessionActor).host(ProducerActor).build();
    system.start().await?;

    // One key per stream life, never used again.
    let session = Session { stream: format!("demo-{}", MessageId::random().as_str()) };
    let producer = Producer { id: session.stream.clone() };
    system.create(&session, &SessionState::default()).await?;
    system.create(&producer, &ProducerState::new(session.clone())).await?;

    let shutdown = CancellationToken::new();
    let app = adapter::router(system.clone(), shutdown.clone()).route("/", get(|| async { Html(CLIENT) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    println!("store:   {}", path.display());
    println!("socket:  ws://{address}/sessions/{}/socket", session.stream);
    println!("browser: http://{address}/?stream={}", session.stream);
    let server = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { axum::serve(listener, app).with_graceful_shutdown(shutdown.cancelled_owned()).await }
    });

    let interrupted = tokio::select! {
        result = emit(&system, &producer, 5) => { result?; false }
        result = tokio::signal::ctrl_c() => { result?; true }
    };
    if !interrupted {
        println!("serving; Ctrl-C stops");
        tokio::signal::ctrl_c().await?;
    }
    shutdown.cancel();
    server.await??;
    system.stop().await;
    store.close().await;
    Ok(())
}

/// Emits `count` frames, one at a time: the producer takes the next only after the session answered the last.
async fn emit(system: &ActorSystem, producer: &Producer, count: u32) -> anyhow::Result<()> {
    for n in 1..=count {
        system.send(producer, ProducerMsg::Emit { body: format!("frame {n}") }).await?;
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let address = cicada::ActorRef::address(producer)?;
            let Some(snapshot) = system.get_actor(&address).await? else { anyhow::bail!("the producer is gone") };
            let state: ProducerState = serde_json::from_value(snapshot.state)?;
            if state.pending.is_none() {
                println!("frame {n}: {:?}", state.last);
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Ok(())
}
