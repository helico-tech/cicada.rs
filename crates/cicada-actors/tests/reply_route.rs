//! Flow and actor callers of the same port, with replies sent in a later turn.

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada::turn::TurnError;
use cicada_actors::{Flow, FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus, JournalEntry, ReplyRoute, Step};
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Serialize, Deserialize)]
struct Job;

impl ActorRef for Job {
    const TYPE: &'static str = "jobs";
    type Message = JobMsg;
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum JobMsg {
    Start { reply: ReplyRoute<JobReply> },
    Finish,
    Invalid { reply: ReplyRoute<u32> },
    Reject { reply: ReplyRoute<JobReply> },
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum JobReply {
    Finished { size: u32 },
}

#[derive(Default, Serialize, Deserialize, JsonSchema)]
struct JobState {
    reply: Option<ReplyRoute<JobReply>>,
}

struct JobActor;

impl Actor for JobActor {
    type Ref = Job;
    type State = JobState;

    async fn handle(&self, turn: &mut Turn<Self>, message: JobMsg) -> HandlerResult {
        match message {
            JobMsg::Start { reply } => turn.state_mut().reply = Some(reply),
            JobMsg::Finish => {
                let reply = turn.state_mut().reply.take().expect("the request was saved");
                reply.send(turn, &JobReply::Finished { size: 5 }).await?;
            }
            JobMsg::Invalid { reply } => {
                let future = reply.send(turn, &5);
                drop(reply);
                assert!(matches!(future.await, Err(TurnError::Invalid(_))));
            }
            JobMsg::Reject { reply } => {
                let error = reply.send(turn, &JobReply::Finished { size: 5 }).await.unwrap_err();
                assert!(matches!(error, TurnError::SendRejected { to, message_type, .. }
                    if to == Job.address().unwrap() && message_type == "Finished"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Caller;

impl ActorRef for Caller {
    const TYPE: &'static str = "caller";
    type Message = CallerMsg;
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CallerMsg {
    Ask { correlation: Option<String> },
    Finished { size: u32 },
}

#[derive(Default, Serialize, Deserialize, JsonSchema)]
struct CallerState {
    size: Option<u32>,
    correlation: Option<String>,
}

struct CallerActor;

impl Actor for CallerActor {
    type Ref = Caller;
    type State = CallerState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CallerMsg) -> HandlerResult {
        match message {
            CallerMsg::Ask { correlation } => {
                let mut reply = ReplyRoute::actor(turn.self_address().clone());
                if let Some(id) = correlation {
                    reply = reply.correlation(id);
                }
                turn.send(&Job, JobMsg::Start { reply })?;
            }
            CallerMsg::Finished { size } => {
                let correlation = turn.message().correlation_id.clone();
                *turn.state_mut() = CallerState { size: Some(size), correlation };
            }
        }
        Ok(())
    }
}

struct Build;

impl Flow for Build {
    const TYPE: &'static str = "build";
    type Input = ();

    async fn run(&self, step: &mut Step, (): ()) -> HandlerResult {
        let request = step.request("start", &Job, |r| JobMsg::Start { reply: r.into() }).await?;
        let JobReply::Finished { size } = step.wait("finished", &request).await?;
        step.run("record", |_| async move { Ok(size) }).await?;
        Ok(())
    }
}

async fn node() -> TestActors {
    let test = TestActors::builder()
        .with(|node| node.host(JobActor).host(CallerActor).host(FlowActor::new(Build)))
        .build()
        .await
        .unwrap();
    test.create(&Job, &JobState::default()).await;
    test.create(&Caller, &CallerState::default()).await;
    test
}

#[tokio::test(start_paused = true)]
async fn an_actor_gets_a_later_reply_in_its_own_family_with_its_correlation_id() {
    let test = node().await;
    let caller = Caller.address().unwrap();
    for (correlation, expected) in [(Some("c1"), "c1"), (None, "finish")] {
        test.send(&Caller, CallerMsg::Ask { correlation: correlation.map(str::to_owned) }).await;
        test.run_until_idle().await;
        assert!(test.state::<_, JobState>(&Job).await.reply.is_some());

        test.system().pause_mailbox(&caller).await.unwrap();
        let finish = test.send_with(&Job, JobMsg::Finish, SendOptions::new().correlation_id("finish")).await;
        test.run_until_idle().await;
        let pending = test.pending(&Caller).await;
        let [reply] = pending.as_slice() else { panic!("expected one pending reply: {pending:?}") };
        assert_eq!(reply.message_type, "Finished");
        assert_eq!(reply.payload, json!({"size": 5}));
        assert_eq!(reply.correlation_id.as_deref(), Some(expected));
        assert_eq!(reply.caused_by.as_ref(), Some(&finish.id));
        assert_eq!(reply.deliver_at, test.now());
        assert!(reply.retry.is_none());
        assert!(reply.retention.is_none());

        test.system().resume_mailbox(&caller).await.unwrap();
        test.run_until_idle().await;
        let received: CallerState = test.state(&Caller).await;
        assert_eq!(received.size, Some(5));
        assert_eq!(received.correlation.as_deref(), Some(expected));
    }
    assert!(test.failures().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_flow_wait_receives_the_ports_reply() {
    let test = node().await;
    let run = FlowRun::<Build>::named("b1");
    test.create(&run, &FlowState::new(())).await;
    test.send(&run, FlowMsg::Continue).await;
    test.run_until_idle().await;
    let waiting: FlowState<()> = test.state(&run).await;
    assert_eq!(waiting.status, FlowStatus::Running);
    assert!(waiting.armed.is_none());
    assert!(matches!(waiting.journal.last(), Some(JournalEntry::Wait { outcome: None, .. })));

    test.send(&Job, JobMsg::Finish).await;
    test.run_until_idle().await;
    let done: FlowState<()> = test.state(&run).await;
    assert_eq!(done.status, FlowStatus::Done);
    assert!(
        matches!(done.journal.last(), Some(JournalEntry::Step { id, result }) if id == "record" && *result == json!(5))
    );
    assert!(test.failures().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_scalar_reply_is_invalid_and_buffers_nothing() {
    let test = node().await;
    let reply = ReplyRoute::actor(Caller.address().unwrap());
    let invalid = test.send(&Job, JobMsg::Invalid { reply }).await;
    assert_eq!(test.run_until_idle().await, 1);
    assert_eq!(test.outcome(&invalid).await.unwrap().status, OutcomeStatus::Done);
    assert!(test.pending(&Caller).await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_reply_whose_type_name_the_caller_lacks_is_rejected() {
    let test = node().await;
    let reply = ReplyRoute::actor(Job.address().unwrap());
    let rejected = test.send(&Job, JobMsg::Reject { reply }).await;
    assert_eq!(test.run_until_idle().await, 1);
    assert_eq!(test.outcome(&rejected).await.unwrap().status, OutcomeStatus::Done);
    assert!(test.pending(&Job).await.is_empty());
}
