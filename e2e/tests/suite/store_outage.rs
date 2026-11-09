//! PostgreSQL is stopped under a running cluster. Every node must answer sends and `/health`
//! with `503` problems (so clients retry), and the cluster must recover by itself when PostgreSQL is back: no message
//! accepted before the outage is lost, and new sends work again.

use std::time::Instant;

use cicada::api::{MessageState, problem_types};
use cicada::store::MessageId;
use cicada_e2e::cluster::{RawResponse, cluster, eventually, path, run_id, uri};
use cicada_e2e::docker::compose;
use cicada_e2e::ordering::send_step;
use cicada_e2e::{Class, step};
use cicada_sample_service::e2e::{Counter, CounterState};
use futures::future::join_all;
use reqwest::Method;

use crate::node_fault::restore;
use crate::{SECOND, secs};

pub fn class() -> Class {
    Class {
        after_each: Some(step!(restore).1),
        ..Class::new(
            "StoreOutageTest",
            vec![step!(store_unavailable_sends_and_health_answer_503_the_cluster_recovers_when_postgresql_is_back)],
        )
    }
}

fn assert_problem(response: &RawResponse, problem_type: &str) {
    let content_type = response.content_type.as_deref().unwrap_or_default();
    assert!(content_type.starts_with("application/problem+json"), "problem+json expected: {content_type} {response}");
    assert_eq!(response.text("/type").as_deref(), Some(problem_type), "{response}");
    assert_eq!(response.json()["status"], 503, "{response}");
}

async fn store_unavailable_sends_and_health_answer_503_the_cluster_recovers_when_postgresql_is_back() {
    let c = cluster();
    let counter = Counter { id: format!("outage-{}", run_id()) };
    c.node1.client.create(&counter, &CounterState::default()).await.unwrap();
    // Accepted before the outage, due while PostgreSQL is down: it must wait for the store, not get lost.
    let accepted = Instant::now();
    let delay = secs(6);
    let before = send_step(&c.nodes(), &counter, 1, 0, delay).await;
    assert_eq!(c.send_only.status(&before.id).await.status, MessageState::Pending);

    compose::stop(compose::POSTGRES).await.unwrap();
    let outage = Instant::now();
    assert!(accepted.elapsed() < delay, "PostgreSQL took too long to stop ({:.1?})", accepted.elapsed());

    // A send refused with 503 was not accepted: the same id is sent again after the recovery.
    let retried = MessageId::random();
    let body = format!(r#"{{"id":"{retried}","type":"Step","payload":{{"seq":2}}}}"#);
    let mailbox = format!("{}/messages", path(&counter));
    // In parallel: a request can wait up to the pool's connection timeout before it fails.
    join_all(c.nodes().into_iter().map(|node| {
        let (body, mailbox) = (&body, &mailbox);
        async move {
            let send = eventually(&format!("{node} refusing sends with 503"), secs(60), async || {
                let response = node.request(Method::POST, mailbox, Some(body)).await?;
                Ok((response.status == 503).then_some(response))
            })
            .await;
            assert_problem(&send, problem_types::STORE_UNAVAILABLE);
            let health = eventually(&format!("{node} unhealthy"), secs(60), async || {
                let response = node.request(Method::GET, "/health", None).await?;
                Ok((response.status == 503).then_some(response))
            })
            .await;
            assert_problem(&health, problem_types::UNHEALTHY);
        }
    }))
    .await;
    // Keep the store down until the delayed message is overdue, so it is claimed only after the recovery.
    if let Some(overdue) = (delay + SECOND).checked_sub(accepted.elapsed()) {
        tokio::time::sleep(overdue).await;
    }
    println!("outage: every node answered 503; PostgreSQL down for {:.1?}", outage.elapsed());

    compose::start(compose::POSTGRES, true).await.unwrap();
    let recovery = Instant::now();
    c.await_healthy(secs(90)).await;
    println!("outage: cluster healthy again {:.1?} after PostgreSQL started", recovery.elapsed());

    let after = eventually("send accepted again", secs(30), async || {
        let response = c.send_only.request(Method::POST, &mailbox, Some(&body)).await?;
        Ok((response.status == 202).then_some(response))
    })
    .await;
    assert_eq!(after.text("/id").as_deref(), Some(retried.as_str()));

    for id in [&before.id, &retried] {
        let status = c.send_only.await_finished_within(id, secs(60)).await;
        assert_eq!(status.status, MessageState::Done, "message {id}");
    }
    let state: CounterState = c.send_only.state(&counter).await;
    assert_eq!(state.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2], "each step handled once, in order");
    assert_eq!(state.out_of_order, 0);
    assert_eq!(state.overlaps, 0);
    assert!(c.send_only.failures(&uri(&counter)).await.is_empty());
}
