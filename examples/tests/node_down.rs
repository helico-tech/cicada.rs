//! Example 22: telling on-call that a node stopped heartbeating.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::runtime::{NodeHealth, NodeInfo};
use cicada::store::NodeId;
use cicada::time::Timestamp;
use cicada_actors::{
    NodeDownMonitorActor, NodeReadError, NodeReadProblem, NodeSnapshot, NodeSnapshotSource, TopicActor,
};
use cicada_examples::node_down::{
    MAX_LINES, Pager, PagerActor, PagerMsg, PagerState, alerts, read_problem, repair, start,
};
use cicada_test::TestActors;

const SECOND: Duration = Duration::from_secs(1);

/// The node table as the test sets it: rows sampled at 00:10, or a failed read.
#[derive(Clone, Default)]
struct Table(Arc<Mutex<Option<Vec<NodeInfo>>>>);

impl Table {
    fn set(&self, rows: Option<Vec<NodeInfo>>) {
        *self.0.lock().unwrap() = rows;
    }
}

impl NodeSnapshotSource for Table {
    async fn snapshot(&self) -> Result<NodeSnapshot, NodeReadError> {
        let rows = self.0.lock().unwrap().clone();
        let nodes = rows.ok_or_else(|| NodeReadError("the store is down".into()))?;
        Ok(NodeSnapshot { as_of: at("2026-03-01T00:10:00Z"), nodes })
    }
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// `web-1`, started at midnight and last seen at `last_seen`.
fn web(last_seen: &str) -> NodeInfo {
    NodeInfo {
        id: NodeId::new("web-1").unwrap(),
        hosted: Vec::new(),
        started_at: at("2026-03-01T00:00:00Z"),
        last_seen: at(last_seen),
        hostname: Some("web-1.internal".into()),
        cicada_version: Some("0.20.2".into()),
        label: Some("shop".into()),
        status: NodeHealth::Alive,
        running_turns: Vec::new(),
    }
}

/// A node with the monitor on `table`, the topic and the pager, set up by `start`; the first poll ran.
async fn node(table: &Table) -> TestActors {
    let source = table.clone();
    let test = TestActors::builder()
        .start_at(at("2026-03-01T00:10:00Z"))
        .with(move |node| node.host(NodeDownMonitorActor::new(source)).host(TopicActor).host(PagerActor))
        .build()
        .await
        .unwrap();
    start(test.system()).await.unwrap();
    test.run_until_idle().await;
    test
}

async fn lines(test: &TestActors) -> Vec<String> {
    test.state::<_, PagerState>(&Pager).await.lines
}

#[tokio::test(start_paused = true)]
async fn a_node_that_stopped_heartbeating_pages_once() {
    let table = Table::default();
    table.set(Some(vec![web("2026-03-01T00:09:00Z")]));
    let test = node(&table).await;
    assert!(lines(&test).await.is_empty(), "silent for a minute: not dead");
    table.set(Some(vec![web("2026-03-01T00:07:30Z")]));
    test.advance_time_by(10 * SECOND).await;
    assert_eq!(lines(&test).await, ["node web-1 (web-1.internal) silent since 2026-03-01T00:07:30Z"]);
    test.advance_time_by(600 * SECOND).await;
    assert_eq!(lines(&test).await.len(), 1, "one silence, one page");
    start(test.system()).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(lines(&test).await.len(), 1, "a repeated start keeps what was alerted");
}

#[tokio::test(start_paused = true)]
async fn a_failed_read_shows_in_last_problem_and_not_in_the_pager() {
    let table = Table::default();
    let test = node(&table).await;
    assert_eq!(read_problem(test.system()).await.unwrap(), Some(NodeReadProblem::Unavailable));
    assert!(lines(&test).await.is_empty(), "a caught read error is no failed turn: the watch says nothing");
    table.set(Some(vec![web("2026-03-01T00:07:30Z")]));
    test.advance_time_by(10 * SECOND).await;
    assert_eq!(read_problem(test.system()).await.unwrap(), None);
    assert_eq!(lines(&test).await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_lost_topic_is_told_to_the_pager_and_repair_delivers_the_page() {
    let table = Table::default();
    table.set(Some(Vec::new()));
    let test = node(&table).await;
    test.system().delete_actor(&cicada::ActorRef::address(&alerts()).unwrap()).await.unwrap();
    table.set(Some(vec![web("2026-03-01T00:07:30Z")]));
    test.advance_time_by(60 * SECOND).await;
    assert_eq!(lines(&test).await, ["monitor Poll failed: Commit"], "the poll failed for good; its timer is lost");
    repair(test.system()).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(lines(&test).await[1..], ["node web-1 (web-1.internal) silent since 2026-03-01T00:07:30Z"]);
}

#[tokio::test(start_paused = true)]
async fn the_pager_keeps_the_latest_lines() {
    let table = Table::default();
    table.set(Some(Vec::new()));
    let test = node(&table).await;
    for i in 0..MAX_LINES + 5 {
        let mut down = web("2026-03-01T00:00:00Z");
        down.id = NodeId::new(format!("n{i}")).unwrap();
        let alert = cicada_actors::NodeDown {
            node: down.id,
            started_at: down.started_at,
            last_seen: down.last_seen,
            detected_at: at("2026-03-01T00:10:00Z"),
            hostname: None,
            cicada_version: None,
            label: None,
        };
        test.send(&Pager, PagerMsg::NodeDown(alert)).await;
    }
    test.run_until_idle().await;
    let kept = lines(&test).await;
    assert_eq!(kept.len(), MAX_LINES);
    assert!(kept[0].starts_with("node n5 (unknown host)"), "{}", kept[0]);
}
