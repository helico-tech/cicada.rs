//! Client and node over a real socket, the API nested under a prefix.

mod common;

use axum::Router;
use cicada::api::MessageState;
use cicada_axum::actor_api;
use cicada_client::ActorClient;
use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_create_send_and_read_over_http() {
    let system = node_with(|b| b.host(AccountActor)).await;
    let base = serve(Router::new().nest("/internal", actor_api(system.clone()))).await;

    // A trailing slash on the base URL is fine.
    let actors = ActorClient::new(format!("{base}/internal/"));
    assert!(actors.health().await.unwrap());
    let ada = account("acme", 1);
    assert!(actors.create(&ada, &owned_by("Ada")).await.unwrap().created);
    let mut receipts = Vec::new();
    for n in 1..=3 {
        receipts.push(actors.send(&ada, AccountMsg::Deposit { amount: n }).await.unwrap());
    }
    eventually(async || {
        for r in &receipts {
            if actors.status(&r.id).await.unwrap().map(|s| s.status) != Some(MessageState::Done) {
                return false;
            }
        }
        true
    })
    .await;
    assert_eq!(actors.state(&ada).await.unwrap(), Some(AccountState { balance: 6, owner: "Ada".into() }));

    let http = reqwest::Client::new();
    assert_eq!(
        ActorClient::with_client(format!("{base}/internal"), http)
            .list_actors("", None, None)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    system.stop().await;
}
