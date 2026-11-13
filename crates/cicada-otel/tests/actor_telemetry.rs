//! The telemetry observer, against the SDK's in-memory exporters: spans, metrics, the trace context of sends, and the
//! linked roots of timers (ADR 0032).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::observer::{ObservedTurn, TurnObserver, TurnStart};
use cicada::prelude::*;
use cicada::runtime::ActorSystemBuilder;
use cicada::{CreateActorResult, WatchTarget};
use cicada_actors::{
    DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, Flow, FlowActor, FlowMsg, FlowRun, FlowState,
    Schedule, Scheduler, SchedulerActor, SchedulerMsg, SchedulerState, Step,
};
use cicada_otel::ActorTelemetry;
use cicada_test::{TestActors, claim_and_abandon};
use opentelemetry::propagation::text_map_propagator::FieldIter;
use opentelemetry::propagation::{Extractor, Injector, TextMapCompositePropagator, TextMapPropagator};
use opentelemetry::trace::{SpanContext, SpanId, SpanKind, Status, TraceContextExt, TraceFlags, TraceId, TraceState};
use opentelemetry::{Context, KeyValue, Value};
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, HistogramDataPoint, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Traced {
    id: String,
}

impl ActorRef for Traced {
    const TYPE: &'static str = "traced";
    type Message = TracedMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum TracedMsg {
    Hop {
        left: u32,
    },
    /// Sends `Hop { left: 0 }` to `b` with its own `traceparent`.
    OwnParent,
    Crash,
    /// Sends `Hop { left: 0 }` to `b` now and, while `left` is above zero, itself `Tick { left: left - 1 }` in a minute.
    Tick {
        left: u32,
    },
    /// Sends `Hop { left: 0 }` to itself now and to `b` in ten seconds.
    Mixed,
    /// Sends `Hop { left: 0 }` to itself in ten seconds, with `own` headers of its own (`own-00`, …) and `headers`.
    Later {
        #[serde(default)]
        own: u32,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    /// Sends `Crash` to itself in ten seconds, with a policy of `attempts` attempts a second apart.
    ArmCrash {
        attempts: u32,
    },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct TracedState {
    hops: u32,
}

const OWN_PARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

type Seen = Arc<Mutex<Vec<BTreeMap<String, String>>>>;

struct TracedActor {
    seen: Seen,
}

impl Actor for TracedActor {
    type Ref = Traced;
    type State = TracedState;

    async fn handle(&self, turn: &mut Turn<Self>, message: TracedMsg) -> HandlerResult {
        match message {
            TracedMsg::Hop { left } => {
                self.seen.lock().unwrap().push(turn.message().headers.clone());
                turn.state_mut().hops += 1;
                if left > 0 {
                    turn.send_self(TracedMsg::Hop { left: left - 1 })?;
                }
                Ok(())
            }
            TracedMsg::OwnParent => {
                let own = SendOptions::new().header("traceparent", OWN_PARENT);
                turn.send_with(&Traced { id: "b".into() }, TracedMsg::Hop { left: 0 }, own)?;
                Ok(())
            }
            TracedMsg::Crash => Err(HandlerError::msg("boom")),
            TracedMsg::Tick { left } => {
                turn.send(&b(), TracedMsg::Hop { left: 0 })?;
                if left > 0 {
                    turn.send_self_with(TracedMsg::Tick { left: left - 1 }, SendOptions::new().delay(MINUTE))?;
                }
                Ok(())
            }
            TracedMsg::Mixed => {
                turn.send_self(TracedMsg::Hop { left: 0 })?;
                turn.send_with(&b(), TracedMsg::Hop { left: 0 }, SendOptions::new().delay(TEN_SECONDS))?;
                Ok(())
            }
            TracedMsg::Later { own, headers } => {
                let mut options = SendOptions::new().delay(TEN_SECONDS);
                for n in 0..own {
                    options = options.header(format!("own-{n:02}"), "o");
                }
                for (name, value) in headers {
                    options = options.header(name, value);
                }
                turn.send_self_with(TracedMsg::Hop { left: 0 }, options)?;
                Ok(())
            }
            TracedMsg::ArmCrash { attempts } => {
                let retry = RetryPolicy::new(attempts, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
                turn.send_self_with(TracedMsg::Crash, SendOptions::new().delay(TEN_SECONDS).retry(retry))?;
                Ok(())
            }
        }
    }
}

struct Telemetry {
    test: TestActors,
    spans: InMemorySpanExporter,
    metrics: InMemoryMetricExporter,
    meter_provider: SdkMeterProvider,
    seen: Seen,
}

impl Telemetry {
    async fn new() -> Self {
        Self::build(TraceContextPropagator::new(), |node| node).await
    }

    /// The traced actor and the telemetry with `propagator`, and then whatever `configure` adds to the node.
    async fn build(
        propagator: impl TextMapPropagator + Send + Sync + 'static,
        configure: impl FnOnce(ActorSystemBuilder) -> ActorSystemBuilder + 'static,
    ) -> Self {
        let spans = InMemorySpanExporter::default();
        let tracer_provider = SdkTracerProvider::builder().with_simple_exporter(spans.clone()).build();
        let metrics = InMemoryMetricExporter::default();
        let meter_provider =
            SdkMeterProvider::builder().with_reader(PeriodicReader::builder(metrics.clone()).build()).build();
        let telemetry = ActorTelemetry::new(&tracer_provider, &meter_provider).with_propagator(propagator);
        let seen = Seen::default();
        let actor = TracedActor { seen: seen.clone() };
        let test = TestActors::builder()
            .with(move |node| configure(node.host(actor).observe(telemetry)))
            .build()
            .await
            .unwrap();
        test.create(&traced(), &TracedState::default()).await;
        test.create(&b(), &TracedState::default()).await;
        Self { test, spans, metrics, meter_provider, seen }
    }

    fn spans(&self) -> Vec<SpanData> {
        self.spans.get_finished_spans().unwrap()
    }

    /// The single data point of the histogram `name`.
    fn histogram(&self, name: &str) -> HistogramDataPoint<f64> {
        self.meter_provider.force_flush().unwrap();
        let exported = self.metrics.get_finished_metrics().unwrap();
        let metric = exported
            .iter()
            .flat_map(|r| r.scope_metrics())
            .flat_map(|s| s.metrics())
            .find(|m| m.name() == name)
            .unwrap_or_else(|| panic!("no metric {name}"));
        let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() else {
            panic!("{name} is not an f64 histogram");
        };
        let points: Vec<_> = histogram.data_points().cloned().collect();
        assert_eq!(points.len(), 1, "{points:?}");
        points.into_iter().next().unwrap()
    }
}

fn attribute<'a>(attributes: impl IntoIterator<Item = &'a KeyValue>, key: &str) -> Option<Value> {
    attributes.into_iter().find(|kv| kv.key.as_str() == key).map(|kv| kv.value.clone())
}

fn str_attr(span: &SpanData, key: &str) -> Option<String> {
    attribute(&span.attributes, key).map(|v| v.as_str().into_owned())
}

fn long_attr(span: &SpanData, key: &str) -> Option<i64> {
    match attribute(&span.attributes, key) {
        Some(Value::I64(v)) => Some(v),
        other => panic!("{key} is not a long: {other:?}"),
    }
}

fn traced() -> Traced {
    Traced { id: "a".into() }
}

fn b() -> Traced {
    Traced { id: "b".into() }
}

const MINUTE: Duration = Duration::from_secs(60);
const TEN_SECONDS: Duration = Duration::from_secs(10);

#[tokio::test(start_paused = true)]
async fn one_consumer_span_per_turn_with_the_messaging_attributes() {
    let t = Telemetry::new().await;
    let msg = t.test.send_with(&traced(), TracedMsg::Hop { left: 0 }, SendOptions::new().correlation_id("c-1")).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    assert_eq!(spans.len(), 1);
    let span = &spans[0];
    assert_eq!(span.name, "process traced");
    assert_eq!(span.span_kind, SpanKind::Consumer);
    assert_eq!(str_attr(span, "messaging.system").as_deref(), Some("cicada"));
    assert_eq!(str_attr(span, "messaging.operation.name").as_deref(), Some("process"));
    assert_eq!(str_attr(span, "messaging.destination.name").as_deref(), Some("traced"));
    assert_eq!(str_attr(span, "messaging.message.id").as_deref(), Some(msg.id.as_str()));
    assert_eq!(str_attr(span, "messaging.message.conversation_id").as_deref(), Some("c-1"));
    assert_eq!(str_attr(span, "cicada.actor").as_deref(), Some("traced/a"));
    assert_eq!(str_attr(span, "cicada.message.type").as_deref(), Some("Hop"));
    assert_eq!(long_attr(span, "cicada.attempt"), Some(1));
    assert_eq!(long_attr(span, "cicada.failures"), Some(0));
    assert_eq!(str_attr(span, "cicada.outcome").as_deref(), Some("done"));
    assert_eq!(long_attr(span, "cicada.sent"), Some(0));
    assert_eq!(span.status, Status::Unset);
    assert_eq!(span.parent_span_id, SpanId::INVALID, "a message without trace headers starts a trace");
}

/// Scenario: A span per turn.
#[tokio::test(start_paused = true)]
async fn a_span_per_turn_and_the_span_of_a_sent_message_is_a_child_of_the_turn_that_sent_it() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::Hop { left: 1 }).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    assert_eq!(spans.len(), 2);
    assert_eq!(str_attr(&spans[0], "cicada.outcome").as_deref(), Some("done"));
    assert_eq!(spans[1].parent_span_id, spans[0].span_context.span_id());
}

#[tokio::test(start_paused = true)]
async fn a_chain_of_turns_is_one_trace_without_the_handlers_passing_headers() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::Hop { left: 2 }).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    assert_eq!(spans.len(), 3);
    assert!(spans.iter().all(|s| s.span_context.trace_id() == spans[0].span_context.trace_id()));
    assert_eq!(spans[1].parent_span_id, spans[0].span_context.span_id());
    assert_eq!(spans[2].parent_span_id, spans[1].span_context.span_id());
    assert_eq!(long_attr(&spans[0], "cicada.sent"), Some(1));
}

#[tokio::test(start_paused = true)]
async fn the_parent_comes_from_the_message_headers_also_when_the_name_is_not_lower_case() {
    let t = Telemetry::new().await;
    let parent = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let options = SendOptions::new().header("Traceparent", parent);
    t.test.send_raw(&traced().address().unwrap(), "Hop", json!({ "left": 0 }), options).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].span_context.trace_id().to_string(), "0af7651916cd43dd8448eb211c80319c");
    assert_eq!(spans[0].parent_span_id.to_string(), "b7ad6b7169203331");
}

/// The `exception` event of `span`, with its message.
fn exception_message(span: &SpanData) -> Option<String> {
    let exception = span.events.iter().find(|e| e.name == "exception")?;
    attribute(&exception.attributes, "exception.message").map(|v| v.as_str().into_owned())
}

/// Scenario: A handler fails and is retried (of the telemetry).
#[tokio::test(start_paused = true)]
async fn a_failed_turn_has_error_status_and_a_retried_and_a_failed_one_record_the_exception() {
    let t = Telemetry::new().await;
    let retry = RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    t.test.send_with(&traced(), TracedMsg::Crash, SendOptions::new().retry(retry)).await;
    t.test.advance_time_by(Duration::from_secs(5)).await;
    let spans = t.spans();
    assert_eq!(spans.len(), 2);
    let (first, second) = (&spans[0], &spans[1]);
    assert_eq!(str_attr(first, "cicada.outcome").as_deref(), Some("rescheduled"));
    assert_eq!(first.status, Status::Unset);
    assert_eq!(exception_message(first).as_deref(), Some("boom"), "the error of the reschedule");
    assert_eq!(str_attr(second, "cicada.outcome").as_deref(), Some("failed"));
    let Status::Error { description } = &second.status else { panic!("not an error: {:?}", second.status) };
    assert!(description.contains("boom"), "{description}");
    assert_eq!(exception_message(second).as_deref(), Some("boom"), "the error of the failure");
    assert_eq!(long_attr(second, "cicada.failures"), Some(1));
}

/// Scenario: State that cannot be decoded (of the telemetry).
#[tokio::test(start_paused = true)]
async fn a_turn_whose_state_cannot_be_decoded_gets_a_span_from_its_claim_with_the_error_and_an_exception() {
    let t = Telemetry::new().await;
    // State that does not decode, planted by a node that does not host the type (it checks only for null).
    let address = cicada::ActorAddress::parse("traced/bad").unwrap();
    let planter = ActorSystem::builder(t.test.store().clone()).build();
    assert_eq!(planter.create_actor(&address, json!("not a state")).await.unwrap(), CreateActorResult::Created);
    t.test.send_raw(&address, "Hop", json!({ "left": 0 }), SendOptions::new()).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(str_attr(&spans[0], "cicada.outcome").as_deref(), Some("failed"));
    assert!(matches!(spans[0].status, Status::Error { .. }));
    let exception = exception_message(&spans[0]).expect("an exception event");
    assert!(exception.starts_with("State cannot be decoded"), "{exception}");
    assert_eq!(long_attr(&spans[0], "cicada.attempt"), Some(1), "the span starts at the claim");
}

#[tokio::test(start_paused = true)]
async fn turn_duration_and_lag_are_recorded_per_type_message_type_and_outcome() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::Hop { left: 1 }).await;
    t.test.run_until_idle().await;
    let duration = t.histogram("cicada.turn.duration");
    assert_eq!(duration.count(), 2);
    assert_eq!(attribute(duration.attributes(), "cicada.outcome"), Some("done".into()));
    assert_eq!(attribute(duration.attributes(), "cicada.actor.type"), Some("traced".into()));
    assert_eq!(attribute(duration.attributes(), "cicada.message.type"), Some("Hop".into()));
    assert_eq!(t.histogram("cicada.message.lag").count(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_sent_message_carries_the_turns_traceparent_only_with_the_observer() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::Hop { left: 1 }).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    let seen = t.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    // The first message came from outside the actor; the second was sent by the first turn.
    assert!(!seen[0].contains_key("traceparent"));
    assert_eq!(seen[1].keys().collect::<Vec<_>>(), vec!["traceparent"]);
    let expected = format!("00-{}-{}-", spans[0].span_context.trace_id(), spans[0].span_context.span_id());
    assert!(seen[1]["traceparent"].starts_with(&expected), "{seen:?}");

    let seen = Seen::default();
    let actor = TracedActor { seen: seen.clone() };
    let test = TestActors::builder().with(move |node| node.host(actor)).build().await.unwrap();
    test.create(&traced(), &TracedState::default()).await;
    test.send(&traced(), TracedMsg::Hop { left: 1 }).await;
    test.run_until_idle().await;
    assert_eq!(*seen.lock().unwrap(), vec![BTreeMap::new(), BTreeMap::new()]);
}

#[tokio::test(start_paused = true)]
async fn a_send_that_sets_its_own_traceparent_keeps_it() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::OwnParent).await;
    t.test.run_until_idle().await;
    let seen = t.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["traceparent"], OWN_PARENT);
    let spans = t.spans();
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[1].span_context.trace_id().to_string(), "0af7651916cd43dd8448eb211c80319c");
    assert_eq!(spans[1].parent_span_id.to_string(), "b7ad6b7169203331");
}

struct Stuck;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StuckRef {
    id: String,
}

impl ActorRef for StuckRef {
    const TYPE: &'static str = "stuck";
    type Message = StuckMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum StuckMsg {
    Wait,
}

impl Actor for Stuck {
    type Ref = StuckRef;
    type State = cicada::NoState;

    async fn handle(&self, _: &mut Turn<Self>, _: StuckMsg) -> HandlerResult {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        Ok(())
    }
}

#[tokio::test]
async fn the_span_of_a_turn_that_never_commits_is_marked_abandoned() {
    let spans = InMemorySpanExporter::default();
    let tracer_provider = SdkTracerProvider::builder().with_simple_exporter(spans.clone()).build();
    let meter_provider = SdkMeterProvider::builder().build();
    let telemetry = ActorTelemetry::new(&tracer_provider, &meter_provider);
    let store = cicada::Store::memory();
    let system = ActorSystem::builder(store)
        .host(Stuck)
        .observe(telemetry)
        .config(cicada::runtime::NodeConfig {
            poll_interval: std::time::Duration::from_millis(10),
            shutdown_cancel_timeout: std::time::Duration::from_secs(2),
            ..Default::default()
        })
        .build();
    system.start().await.unwrap();
    let stuck = StuckRef { id: "s".into() };
    system.create(&stuck, &cicada::NoState {}).await.unwrap();
    system.send(&stuck, StuckMsg::Wait).await.unwrap();
    while system.running_turns() == 0 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    // The node stops at once: the turn is cancelled and its lease released; it never commits.
    system.stop_with_grace(std::time::Duration::ZERO).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let span = loop {
        if let Some(span) = spans.get_finished_spans().unwrap().into_iter().next() {
            break span;
        }
        assert!(std::time::Instant::now() < deadline, "no span exported");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert_eq!(str_attr(&span, "cicada.outcome").as_deref(), Some("abandoned"));
    assert_eq!(span.status, Status::Unset);
}

/// Scenario: A turn that commits nothing.
#[tokio::test(start_paused = true)]
async fn the_span_of_a_turn_that_loses_its_lease_ends_abandoned_without_an_error() {
    let spans = InMemorySpanExporter::default();
    let tracer_provider = SdkTracerProvider::builder().with_simple_exporter(spans.clone()).build();
    let meter_provider = SdkMeterProvider::builder().build();
    let telemetry = ActorTelemetry::new(&tracer_provider, &meter_provider);
    let test = TestActors::builder().with(move |node| node.host(Stuck).observe(telemetry)).build().await.unwrap();
    let stuck = StuckRef { id: "s".into() };
    test.create(&stuck, &cicada::NoState {}).await;
    test.send(&stuck, StuckMsg::Wait).await;
    let system = test.system().clone();
    let runner = tokio::spawn(async move { system.run_due_turns(1).await.unwrap() });
    tokio::time::sleep(Duration::from_secs(1)).await;
    // Deleting the actor ends the lease of its turn: the next renewal finds it lost.
    test.system().delete_actor(&stuck.address().unwrap()).await.unwrap();
    assert_eq!(runner.await.unwrap(), 1);
    let spans = spans.get_finished_spans().unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(str_attr(&spans[0], "cicada.outcome").as_deref(), Some("abandoned"));
    assert_eq!(spans[0].status, Status::Unset);
    assert_eq!(exception_message(&spans[0]), None);
}

// ---- timers start linked roots (ADR 0032) ----

/// The span context a span links to; asserts that it has at most one link.
fn link(span: &SpanData) -> Option<SpanContext> {
    let links = &span.links.links;
    assert!(links.len() <= 1, "{links:?}");
    links.first().map(|l| l.span_context.clone())
}

fn is_root(span: &SpanData) -> bool {
    span.parent_span_id == SpanId::INVALID
}

/// Asserts that `span` is a root with one link to `sender`, in a trace of its own.
fn assert_linked_root(span: &SpanData, sender: &SpanData) {
    assert!(is_root(span), "{span:?}");
    assert_ne!(span.span_context.trace_id(), sender.span_context.trace_id(), "a trace of its own");
    let link = link(span).expect("a link");
    assert_eq!((link.trace_id(), link.span_id()), (sender.span_context.trace_id(), sender.span_context.span_id()));
}

/// Asserts that `span` is a child of `parent`, without a link.
fn assert_child(span: &SpanData, parent: &SpanData) {
    assert_eq!(span.span_context.trace_id(), parent.span_context.trace_id());
    assert_eq!(span.parent_span_id, parent.span_context.span_id());
    assert_eq!(link(span), None);
}

/// The spans of the turns of `actor` with messages of `message_type`, in the order they ended.
fn spans_of(spans: &[SpanData], actor: &str, message_type: &str) -> Vec<SpanData> {
    spans
        .iter()
        .filter(|s| {
            str_attr(s, "cicada.actor").as_deref() == Some(actor)
                && str_attr(s, "cicada.message.type").as_deref() == Some(message_type)
        })
        .cloned()
        .collect()
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect()
}

fn later(pairs: &[(&str, &str)]) -> TracedMsg {
    TracedMsg::Later { own: 0, headers: headers(pairs) }
}

/// The context of [`OWN_PARENT`].
fn own_parent() -> (String, String) {
    ("0af7651916cd43dd8448eb211c80319c".into(), "b7ad6b7169203331".into())
}

fn ids(cx: &SpanContext) -> (String, String) {
    (cx.trace_id().to_string(), cx.span_id().to_string())
}

/// Scenario: A recurring timer starts linked roots.
#[tokio::test(start_paused = true)]
async fn a_recurring_timer_starts_a_linked_root_per_tick_and_the_work_of_a_tick_is_in_its_trace() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::Tick { left: 2 }).await;
    t.test.advance_time_by(Duration::from_secs(150)).await;
    let spans = t.spans();
    let ticks = spans_of(&spans, "traced/a", "Tick");
    let work = spans_of(&spans, "traced/b", "Hop");
    assert_eq!((ticks.len(), work.len()), (3, 3), "{spans:?}");
    assert!(is_root(&ticks[0]) && link(&ticks[0]).is_none(), "sent from outside");
    assert_linked_root(&ticks[1], &ticks[0]);
    assert_linked_root(&ticks[2], &ticks[1]);
    assert_ne!(ticks[0].span_context.trace_id(), ticks[2].span_context.trace_id());
    for (tick, work) in ticks.iter().zip(&work) {
        assert_child(work, tick);
    }
    let seen = t.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|h| h.contains_key("traceparent") && !h.keys().any(|k| k.starts_with("cicada-link-"))));
}

/// Scenario: Immediate self-sends and delayed sends to others remain children.
#[tokio::test(start_paused = true)]
async fn an_immediate_send_to_itself_and_a_delayed_send_to_another_actor_stay_children() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::Mixed).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "Mixed")[0];
    let to_self = &spans_of(&spans, "traced/a", "Hop")[0];
    let to_other = &spans_of(&spans, "traced/b", "Hop")[0];
    assert_child(to_self, sender);
    assert_child(to_other, sender);
    let seen = t.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|h| h.keys().collect::<Vec<_>>() == vec!["traceparent"]), "{seen:?}");
}

/// Scenario: A timer may have an explicit parent and a link.
#[tokio::test(start_paused = true)]
async fn a_timer_with_its_own_traceparent_is_its_child_and_links_to_the_sender() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), later(&[("traceparent", OWN_PARENT)])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "Later")[0];
    let timer = &spans_of(&spans, "traced/a", "Hop")[0];
    assert_eq!((timer.span_context.trace_id().to_string(), timer.parent_span_id.to_string()), own_parent());
    assert_eq!(ids(&link(timer).unwrap()), ids(&sender.span_context));
    let seen = t.seen.lock().unwrap()[0].clone();
    assert_eq!(seen.keys().collect::<Vec<_>>(), vec!["cicada-link-traceparent", "traceparent"]);
}

/// Adds `traceparent` and `audit` for every send, after the telemetry.
struct Parenting;

impl TurnObserver for Parenting {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        turn.add_send_header("traceparent", OWN_PARENT);
        turn.add_send_header("audit", "on");
        Box::new(())
    }
}

/// Scenario: A timer may have an explicit parent and a link (from another observer).
#[tokio::test(start_paused = true)]
async fn another_observer_can_give_a_timer_a_parent_and_the_timer_keeps_the_link_and_its_other_headers() {
    let t = Telemetry::build(TraceContextPropagator::new(), |node| node.observe(Parenting)).await;
    t.test.send(&traced(), later(&[])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "Later")[0];
    let timer = &spans_of(&spans, "traced/a", "Hop")[0];
    assert_eq!((timer.span_context.trace_id().to_string(), timer.parent_span_id.to_string()), own_parent());
    assert_eq!(ids(&link(timer).unwrap()), ids(&sender.span_context));
    assert_eq!(t.seen.lock().unwrap()[0].get("audit").map(String::as_str), Some("on"));
}

/// Scenario: The all-zero workaround remains valid.
#[tokio::test(start_paused = true)]
async fn a_timer_with_the_all_zero_traceparent_is_a_root_linked_to_the_sender() {
    let zero = "00-00000000000000000000000000000000-0000000000000000-00";
    let t = Telemetry::new().await;
    t.test.send(&traced(), later(&[("traceparent", zero)])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "Later")[0];
    let timer = &spans_of(&spans, "traced/a", "Hop")[0];
    assert_linked_root(timer, sender);
    assert_eq!(str_attr(timer, "cicada.outcome").as_deref(), Some("done"));
}

/// Scenario: Explicit link fields win by exact name.
#[tokio::test(start_paused = true)]
async fn a_timers_own_link_field_wins_and_of_two_cases_the_exact_name_does() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), later(&[("cicada-link-traceparent", OWN_PARENT)])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let timer = &spans_of(&t.spans(), "traced/a", "Hop")[0];
    assert!(is_root(timer));
    assert_eq!(ids(&link(timer).unwrap()), own_parent(), "the caller's value");

    // Another case is another core key: the message carries both, and the exact lower-case name wins.
    let t = Telemetry::new().await;
    t.test.send(&traced(), later(&[("Cicada-Link-Traceparent", OWN_PARENT)])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "Later")[0];
    let timer = &spans_of(&spans, "traced/a", "Hop")[0];
    let seen = t.seen.lock().unwrap()[0].clone();
    assert!(seen.contains_key("Cicada-Link-Traceparent") && seen.contains_key("cicada-link-traceparent"));
    assert_linked_root(timer, sender);
}

/// Sends `Hop { left: 0 }` to `traced/a` from outside with `pairs` as its headers, and returns the span of its turn.
async fn span_of_hop_with(pairs: &[(&str, &str)]) -> SpanData {
    let t = Telemetry::new().await;
    let options = SendOptions { headers: headers(pairs), ..SendOptions::default() };
    t.test.send_with(&traced(), TracedMsg::Hop { left: 0 }, options).await;
    t.test.run_until_idle().await;
    let span = spans_of(&t.spans(), "traced/a", "Hop").remove(0);
    assert_eq!(str_attr(&span, "cicada.outcome").as_deref(), Some("done"));
    span
}

/// Scenario: Link metadata is missing or malformed.
#[tokio::test(start_paused = true)]
async fn a_missing_or_malformed_link_is_left_out_and_a_valid_plain_parent_still_counts() {
    for pairs in [
        vec![],
        vec![("cicada-link-traceparent", "garbage")],
        vec![("cicada-link-tracestate", "k=v")],
        vec![("cicada-link-traceparent", "00-00000000000000000000000000000000-0000000000000000-01")],
    ] {
        let span = span_of_hop_with(&pairs).await;
        assert!(is_root(&span) && link(&span).is_none(), "{pairs:?}: {span:?}");
    }
    let span = span_of_hop_with(&[("cicada-link-traceparent", "garbage"), ("traceparent", OWN_PARENT)]).await;
    assert_eq!((span.span_context.trace_id().to_string(), span.parent_span_id.to_string()), own_parent());
    assert_eq!(link(&span), None);
}

/// Scenario: Link metadata is missing or malformed (inside an ambient trace).
#[tokio::test(start_paused = true)]
async fn a_turn_without_a_parent_is_a_root_inside_an_ambient_trace() {
    let ambient = SpanContext::new(
        TraceId::from_hex("11111111111111111111111111111111").unwrap(),
        SpanId::from_hex("2222222222222222").unwrap(),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    let t = Telemetry::new().await;
    let _guard = Context::new().with_remote_span_context(ambient).attach();
    t.test.send(&traced(), later(&[("cicada-link-tracestate", "k=v")])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "Later")[0];
    assert!(is_root(sender) && link(sender).is_none(), "{sender:?}");
    let timer = &spans_of(&spans, "traced/a", "Hop")[0];
    assert_linked_root(timer, sender);
}

/// Scenario: Header limits can omit telemetry without changing actor behaviour.
#[tokio::test(start_paused = true)]
async fn a_full_timer_goes_without_its_link_and_still_succeeds() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::Later { own: 64, headers: BTreeMap::new() }).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let timer = &spans_of(&t.spans(), "traced/a", "Hop")[0];
    assert!(is_root(timer) && link(timer).is_none());
    assert_eq!(str_attr(timer, "cicada.outcome").as_deref(), Some("done"));
    let seen = t.seen.lock().unwrap()[0].clone();
    assert_eq!(seen.len(), 64);
    assert!(!seen.keys().any(|k| k.starts_with("cicada-link-")), "{seen:?}");
}

/// Scenario: Case-insensitive link extraction.
#[tokio::test(start_paused = true)]
async fn a_link_field_in_another_case_is_read() {
    let span = span_of_hop_with(&[("Cicada-Link-Traceparent", OWN_PARENT)]).await;
    assert!(is_root(&span));
    assert_eq!(ids(&link(&span).unwrap()), own_parent());
}

/// Writes and reads the trace and span id of a context as `<trace id>-<span id>` under its one header.
#[derive(Debug)]
struct Named {
    fields: Vec<String>,
}

impl Named {
    fn new(key: impl Into<String>) -> Self {
        Self { fields: vec![key.into()] }
    }
}

impl TextMapPropagator for Named {
    fn inject_context(&self, cx: &Context, injector: &mut dyn Injector) {
        let span = cx.span();
        let sc = span.span_context();
        if sc.is_valid() {
            injector.set(&self.fields[0], format!("{}-{}", sc.trace_id(), sc.span_id()));
        }
    }

    fn extract_with_context(&self, cx: &Context, extractor: &dyn Extractor) -> Context {
        let parsed = extractor.get(&self.fields[0]).and_then(|v| {
            let (trace, span) = v.split_once('-')?;
            Some((TraceId::from_hex(trace).ok()?, SpanId::from_hex(span).ok()?))
        });
        match parsed {
            Some((trace, span)) => cx.with_remote_span_context(SpanContext::new(
                trace,
                span,
                TraceFlags::SAMPLED,
                true,
                TraceState::default(),
            )),
            None => cx.clone(),
        }
    }

    fn fields(&self) -> FieldIter<'_> {
        FieldIter::new(&self.fields)
    }
}

/// Scenario: A custom propagator determines both representations.
#[tokio::test(start_paused = true)]
async fn a_custom_propagator_writes_its_own_header_and_its_prefixed_form_on_timers() {
    let t = Telemetry::build(Named::new("x-trace"), |node| node).await;
    t.test.send(&traced(), TracedMsg::Mixed).await;
    t.test.send(&traced(), later(&[])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let mixed = &spans_of(&spans, "traced/a", "Mixed")[0];
    let sender = &spans_of(&spans, "traced/a", "Later")[0];
    let hops = spans_of(&spans, "traced/a", "Hop");
    assert_eq!(hops.len(), 2);
    assert_child(&hops[0], mixed);
    assert_linked_root(&hops[1], sender);
    let seen = t.seen.lock().unwrap().clone();
    let mut keys: Vec<Vec<&str>> = seen.iter().map(|h| h.keys().map(String::as_str).collect()).collect();
    keys.sort();
    // The two sends of Mixed (to itself now, to b later) and the timer.
    assert_eq!(keys, vec![vec!["cicada-link-x-trace"], vec!["x-trace"], vec!["x-trace"]]);
}

/// Scenario: A custom propagator determines both representations (composite).
#[tokio::test(start_paused = true)]
async fn a_composite_propagator_gives_one_link() {
    let composite =
        TextMapCompositePropagator::new(vec![Box::new(TraceContextPropagator::new()), Box::new(Named::new("x-trace"))]);
    let t = Telemetry::build(composite, |node| node).await;
    t.test.send(&traced(), later(&[])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "Later")[0];
    let timer = &spans_of(&spans, "traced/a", "Hop")[0];
    assert_eq!(timer.links.links.len(), 1);
    assert_linked_root(timer, sender);
    let seen = t.seen.lock().unwrap()[0].clone();
    assert_eq!(seen.keys().collect::<Vec<_>>(), vec!["cicada-link-traceparent", "cicada-link-x-trace"]);
}

/// Scenario: Header limits can omit telemetry without changing actor behaviour (a long name).
#[tokio::test(start_paused = true)]
async fn a_propagator_key_too_long_with_the_prefix_is_left_out_of_timers_only() {
    let key = "k".repeat(195);
    let t = Telemetry::build(Named::new(key.clone()), |node| node).await;
    t.test.send(&traced(), TracedMsg::Mixed).await;
    t.test.send(&traced(), later(&[])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let mixed = &spans_of(&spans, "traced/a", "Mixed")[0];
    let hops = spans_of(&spans, "traced/a", "Hop");
    assert_child(&hops[0], mixed);
    assert!(is_root(&hops[1]) && link(&hops[1]).is_none());
    assert_eq!(str_attr(&hops[1], "cicada.outcome").as_deref(), Some("done"));
    let seen = t.seen.lock().unwrap().clone();
    // The two sends of Mixed carry the long key; the timer carries nothing.
    assert_eq!(seen.iter().filter(|h| h.contains_key(&key)).count(), 2);
    assert_eq!(seen.iter().filter(|h| h.is_empty()).count(), 1);
}

/// Scenario: Policy retries and lost claims retain the sender link.
#[tokio::test(start_paused = true)]
async fn each_attempt_of_a_timer_is_a_root_linked_to_its_sender() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::ArmCrash { attempts: 3 }).await;
    t.test.advance_time_by(Duration::from_secs(20)).await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "ArmCrash")[0];
    let attempts = spans_of(&spans, "traced/a", "Crash");
    assert_eq!(attempts.len(), 3);
    for attempt in &attempts {
        assert_linked_root(attempt, sender);
    }
    assert_ne!(attempts[0].span_context.trace_id(), attempts[1].span_context.trace_id());
    assert_ne!(attempts[1].span_context.trace_id(), attempts[2].span_context.trace_id());
    let outcomes: Vec<_> = attempts.iter().map(|s| str_attr(s, "cicada.outcome").unwrap()).collect();
    assert_eq!(outcomes, vec!["rescheduled", "rescheduled", "failed"]);
}

/// Scenario: Policy retries and lost claims retain the sender link (a lost claim).
#[tokio::test(start_paused = true)]
async fn a_timer_run_again_after_a_lost_claim_is_a_root_linked_to_its_sender() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), later(&[])).await;
    t.test.run_until_idle().await;
    tokio::time::advance(Duration::from_secs(11)).await;
    let types = [cicada::TypeName::parse("traced").unwrap()].into();
    let abandoned = claim_and_abandon(t.test.store(), &types, Duration::from_secs(5), 1).await.unwrap();
    assert_eq!(abandoned.len(), 1);
    t.test.advance_time_by(Duration::from_secs(10)).await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "Later")[0];
    let timer = &spans_of(&spans, "traced/a", "Hop")[0];
    assert_linked_root(timer, sender);
    assert_eq!(long_attr(timer, "cicada.attempt"), Some(2));
}

/// Scenario: Operator retry and deliver-now preserve trace headers.
#[tokio::test(start_paused = true)]
async fn an_operator_retry_and_a_deliver_now_keep_the_link_of_a_timer() {
    let t = Telemetry::new().await;
    t.test.send(&traced(), TracedMsg::ArmCrash { attempts: 1 }).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let failed = spans_of(&t.spans(), "traced/a", "Crash").remove(0);
    let failed_id = str_attr(&failed, "messaging.message.id").unwrap();
    let retried = t.test.system().retry_message(&cicada::store::MessageId::new(failed_id.clone()).unwrap()).await;
    assert!(retried.is_ok(), "{retried:?}");
    t.test.run_until_idle().await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "ArmCrash")[0];
    let crashes = spans_of(&spans, "traced/a", "Crash");
    assert_eq!(crashes.len(), 2);
    assert_ne!(str_attr(&crashes[1], "messaging.message.id").unwrap(), failed_id, "a new message id");
    assert_linked_root(&crashes[1], sender);
    assert_ne!(crashes[1].span_context.trace_id(), crashes[0].span_context.trace_id());

    let t = Telemetry::new().await;
    t.test.send(&traced(), later(&[])).await;
    t.test.run_until_idle().await;
    let timer = t.test.pending(&traced()).await.remove(0);
    t.test.system().deliver_now(&timer.id).await.unwrap();
    t.test.run_until_idle().await;
    let spans = t.spans();
    let sender = &spans_of(&spans, "traced/a", "Later")[0];
    assert_linked_root(&spans_of(&spans, "traced/a", "Hop")[0], sender);
}

/// Scenario: An older queued timer retains its parent until rearmed.
#[tokio::test(start_paused = true)]
async fn a_timer_queued_with_plain_headers_keeps_its_parent_until_it_rearms() {
    let t = Telemetry::new().await;
    let options = SendOptions::new().delay(MINUTE).header("traceparent", OWN_PARENT);
    t.test.send_with(&traced(), TracedMsg::Tick { left: 1 }, options).await;
    t.test.advance_time_by(Duration::from_secs(150)).await;
    let ticks = spans_of(&t.spans(), "traced/a", "Tick");
    assert_eq!(ticks.len(), 2);
    assert_eq!((ticks[0].span_context.trace_id().to_string(), ticks[0].parent_span_id.to_string()), own_parent());
    assert_linked_root(&ticks[1], &ticks[0]);
}

/// The telemetry observer as it was before 0.16.8, frozen: the parent from the plain headers (exact name first, then
/// any case), a span from it, and the span's context on every send. It knows nothing of links.
struct LegacyTelemetry {
    tracer: opentelemetry_sdk::trace::SdkTracer,
}

/// The legacy reader of message headers.
struct LegacyReader<'a>(&'a BTreeMap<String, String>);

impl Extractor for LegacyReader<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .get(key)
            .or_else(|| self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v))
            .map(String::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

struct LegacySpan(Context);

impl ObservedTurn for LegacySpan {
    fn turn_ended(self: Box<Self>, _end: &cicada::observer::TurnEnd<'_>) {
        self.0.span().end();
    }
}

impl TurnObserver for LegacyTelemetry {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        use opentelemetry::trace::Tracer;
        let propagator = TraceContextPropagator::new();
        let parent = propagator.extract_with_context(&Context::new(), &LegacyReader(&turn.message().headers));
        let span = self
            .tracer
            .span_builder(format!("process {}", turn.type_name()))
            .with_kind(SpanKind::Consumer)
            .start_with_context(&self.tracer, &parent);
        let cx = Context::new().with_span(span);
        let mut headers = std::collections::HashMap::new();
        propagator.inject_context(&cx, &mut headers);
        for (name, value) in headers.into_iter().filter(|(_, v)| !v.is_empty()) {
            turn.add_send_header(name, value);
        }
        Box::new(LegacySpan(cx))
    }
}

/// Scenario: An older queued timer retains its parent until rearmed (an old node reads the new headers).
#[tokio::test(start_paused = true)]
async fn an_old_node_ignores_the_link_headers_of_a_new_timer_and_starts_a_root_without_a_link() {
    // The headers a new node writes on a timer.
    let t = Telemetry::new().await;
    t.test.send(&traced(), later(&[])).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let generated = t.seen.lock().unwrap()[0].clone();
    assert_eq!(generated.keys().collect::<Vec<_>>(), vec!["cicada-link-traceparent"]);

    // An old node handles a message with those headers.
    use opentelemetry::trace::TracerProvider;
    let spans = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder().with_simple_exporter(spans.clone()).build();
    let legacy = LegacyTelemetry { tracer: provider.tracer("legacy") };
    let seen = Seen::default();
    let actor = TracedActor { seen: seen.clone() };
    let old = TestActors::builder().with(move |node| node.host(actor).observe(legacy)).build().await.unwrap();
    old.create(&traced(), &TracedState::default()).await;
    let options = SendOptions { headers: generated.clone(), ..SendOptions::default() };
    let message = old.send_with(&traced(), TracedMsg::Hop { left: 0 }, options).await;
    old.run_until_idle().await;
    assert_eq!(old.outcome(&message).await.unwrap().status, cicada::store::OutcomeStatus::Done);
    assert_eq!(*seen.lock().unwrap(), vec![generated], "handled with the new headers");
    let spans = spans.get_finished_spans().unwrap();
    assert_eq!(spans.len(), 1);
    assert!(is_root(&spans[0]), "{:?}", spans[0]);
    assert!(spans[0].links.links.is_empty(), "{:?}", spans[0].links);
}

/// Scenario: Notifications and redrive use their existing header rules.
#[tokio::test(start_paused = true)]
async fn a_notification_is_a_root_and_a_redrive_is_a_child_of_the_redriving_turn_without_the_link() {
    let t = Telemetry::build(TraceContextPropagator::new(), |node| node.host(DeadLettersActor::default())).await;
    t.test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    t.test.system().watch(&WatchTarget::Every, &DeadLetters::default().address().unwrap()).await.unwrap();
    t.test.send(&traced(), TracedMsg::ArmCrash { attempts: 1 }).await;
    t.test.advance_time_by(Duration::from_secs(15)).await;
    let spans = t.spans();
    let failed = spans_of(&spans, "traced/a", "Crash").remove(0);
    let notified = spans_of(&spans, "cicada.dead-letters/_", "Failed").remove(0);
    assert!(is_root(&notified) && link(&notified).is_none(), "{notified:?}");
    let message_id = cicada::store::MessageId::new(str_attr(&failed, "messaging.message.id").unwrap()).unwrap();
    t.test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id }).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    let redrive = &spans_of(&spans, "cicada.dead-letters/_", "Redrive")[0];
    let redriven = &spans_of(&spans, "traced/a", "Crash")[1];
    assert_child(redriven, redrive);
}

// ---- the ready-made flows and deadlines ----

/// Runs a step, sleeps zero, runs a step, sleeps a minute and runs a step.
struct Napping;

impl Flow for Napping {
    const TYPE: &'static str = "flows.napping";
    type Input = u32;

    async fn run(&self, step: &mut Step, _input: u32) -> HandlerResult {
        step.run("a", |_| async { Ok::<_, HandlerError>(1) }).await?;
        step.sleep("zero", Duration::ZERO).await?;
        step.run("b", |_| async { Ok::<_, HandlerError>(2) }).await?;
        step.sleep("nap", MINUTE).await?;
        step.run("c", |_| async { Ok::<_, HandlerError>(3) }).await?;
        Ok(())
    }
}

/// Scenario: Flow sleeps and scheduler deadlines follow the same rule.
#[tokio::test(start_paused = true)]
async fn a_flows_sleeps_start_linked_roots_and_its_immediate_continuations_stay_in_the_trace() {
    let t = Telemetry::build(TraceContextPropagator::new(), |node| node.host(FlowActor::new(Napping))).await;
    let run = FlowRun::<Napping>::named("r");
    t.test.create(&run, &FlowState::new(0_u32)).await;
    t.test.send(&run, FlowMsg::Continue).await;
    t.test.advance_time_by(Duration::from_secs(90)).await;
    let turns = spans_of(&t.spans(), "flows.napping/r", "Continue");
    // The poke, step a, the zero sleep, step b, the nap, step c, and the replay that completes the run.
    assert_eq!(turns.len(), 7, "{turns:?}");
    assert_child(&turns[1], &turns[0]);
    assert_child(&turns[2], &turns[1]);
    assert_linked_root(&turns[3], &turns[2]); // woken from the zero sleep
    assert_child(&turns[4], &turns[3]);
    assert_linked_root(&turns[5], &turns[4]); // woken from the nap
    assert_child(&turns[6], &turns[5]);
}

/// Scenario: Flow sleeps and scheduler deadlines follow the same rule (the scheduler).
#[tokio::test(start_paused = true)]
async fn a_schedulers_tick_starts_a_linked_root_and_its_run_and_delivery_stay_in_that_trace() {
    let t = Telemetry::build(TraceContextPropagator::new(), |node| node.host(SchedulerActor)).await;
    let scheduler = Scheduler::named("s");
    let state = SchedulerState::new(Schedule::every(60).unwrap(), &b(), TracedMsg::Hop { left: 0 }).unwrap();
    t.test.create(&scheduler, &state).await;
    t.test.send(&scheduler, SchedulerMsg::Start).await;
    t.test.advance_time_by(Duration::from_secs(150)).await;
    let spans = t.spans();
    let start = &spans_of(&spans, "cicada.scheduler/s", "Start")[0];
    let ticks = spans_of(&spans, "cicada.scheduler/s", "Tick");
    let runs = spans_of(&spans, "cicada.scheduler/s", "Run");
    let deliveries = spans_of(&spans, "traced/b", "Hop");
    assert_eq!((ticks.len(), runs.len(), deliveries.len()), (2, 2, 2), "{spans:?}");
    assert_linked_root(&ticks[0], start);
    assert_linked_root(&ticks[1], &ticks[0]);
    for ((tick, run), delivery) in ticks.iter().zip(&runs).zip(&deliveries) {
        assert_child(run, tick);
        assert_child(delivery, run);
    }
}
