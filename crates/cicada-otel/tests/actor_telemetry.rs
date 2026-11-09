//! The telemetry plugin, against the SDK's in-memory exporters.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::prelude::*;
use cicada::store::Store;
use cicada_otel::{ActorTelemetry, trace_headers};
use cicada_test::TestActors;
use opentelemetry::trace::{SpanId, SpanKind, Status};
use opentelemetry::{KeyValue, Value};
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
    const TEMPLATE: &'static str = "actor://traced/{id}";
    type Message = TracedMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum TracedMsg {
    Hop { left: u32 },
    Crash,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct TracedState {
    hops: u32,
}

type Seen = Arc<Mutex<Vec<BTreeMap<String, String>>>>;

struct TracedActor {
    seen: Seen,
}

impl Actor for TracedActor {
    type Ref = Traced;
    type State = TracedState;
    type Message = TracedMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: TracedMsg) -> HandlerResult {
        match message {
            TracedMsg::Hop { left } => {
                self.seen.lock().unwrap().push(trace_headers(turn));
                turn.state_mut().hops += 1;
                if left > 0 {
                    let options = SendOptions { headers: trace_headers(turn), ..SendOptions::default() };
                    turn.send_self_with(TracedMsg::Hop { left: left - 1 }, options)?;
                }
                Ok(())
            }
            TracedMsg::Crash => Err(HandlerError::msg("boom")),
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
        let spans = InMemorySpanExporter::default();
        let tracer_provider = SdkTracerProvider::builder().with_simple_exporter(spans.clone()).build();
        let metrics = InMemoryMetricExporter::default();
        let meter_provider =
            SdkMeterProvider::builder().with_reader(PeriodicReader::builder(metrics.clone()).build()).build();
        let plugin =
            ActorTelemetry::new(&tracer_provider, &meter_provider).with_propagator(TraceContextPropagator::new());
        let seen = Seen::default();
        let actor = TracedActor { seen: seen.clone() };
        let test = TestActors::builder().with(move |node| node.host(actor).install(plugin)).build().await.unwrap();
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

#[tokio::test(start_paused = true)]
async fn one_consumer_span_per_turn_with_the_messaging_attributes() {
    let t = Telemetry::new().await;
    t.test.create(&traced(), &TracedState::default()).await;
    let msg = t.test.send_with(&traced(), TracedMsg::Hop { left: 0 }, SendOptions::new().correlation_id("c-1")).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    assert_eq!(spans.len(), 1);
    let span = &spans[0];
    assert_eq!(span.name, "process actor://traced/{id}");
    assert_eq!(span.span_kind, SpanKind::Consumer);
    assert_eq!(str_attr(span, "messaging.system").as_deref(), Some("cicada"));
    assert_eq!(str_attr(span, "messaging.operation.name").as_deref(), Some("process"));
    assert_eq!(str_attr(span, "messaging.destination.name").as_deref(), Some("actor://traced/{id}"));
    assert_eq!(str_attr(span, "messaging.message.id").as_deref(), Some(msg.id.as_str()));
    assert_eq!(str_attr(span, "messaging.message.conversation_id").as_deref(), Some("c-1"));
    assert_eq!(str_attr(span, "cicada.actor").as_deref(), Some("actor://traced/a"));
    assert_eq!(str_attr(span, "cicada.message.type").as_deref(), Some("Hop"));
    assert_eq!(long_attr(span, "cicada.attempt"), Some(1));
    assert_eq!(long_attr(span, "cicada.failures"), Some(0));
    assert_eq!(str_attr(span, "cicada.outcome").as_deref(), Some("done"));
    assert_eq!(long_attr(span, "cicada.sent"), Some(0));
    assert_eq!(span.status, Status::Unset);
    assert_eq!(span.parent_span_id, SpanId::INVALID, "a message without trace headers starts a trace");
}

#[tokio::test(start_paused = true)]
async fn trace_headers_make_the_next_turn_a_child_so_a_chain_is_one_trace() {
    let t = Telemetry::new().await;
    t.test.create(&traced(), &TracedState::default()).await;
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
    t.test.create(&traced(), &TracedState::default()).await;
    let options = SendOptions::new().header("Traceparent", parent);
    t.test.send_raw(&traced().address().unwrap(), "Hop", json!({ "left": 0 }), options).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].span_context.trace_id().to_string(), "0af7651916cd43dd8448eb211c80319c");
    assert_eq!(spans[0].parent_span_id.to_string(), "b7ad6b7169203331");
}

#[tokio::test(start_paused = true)]
async fn a_failed_turn_has_error_status_a_retried_one_records_the_exception() {
    let t = Telemetry::new().await;
    t.test.create(&traced(), &TracedState::default()).await;
    let retry = RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    t.test.send_with(&traced(), TracedMsg::Crash, SendOptions::new().retry(retry)).await;
    t.test.advance_time_by(Duration::from_secs(5)).await;
    let spans = t.spans();
    assert_eq!(spans.len(), 2);
    let (first, second) = (&spans[0], &spans[1]);
    assert_eq!(str_attr(first, "cicada.outcome").as_deref(), Some("rescheduled"));
    assert_eq!(first.status, Status::Unset);
    let exception = first.events.iter().find(|e| e.name == "exception").expect("an exception event");
    assert!(attribute(&exception.attributes, "exception.message").unwrap().as_str().contains("boom"));
    assert_eq!(str_attr(second, "cicada.outcome").as_deref(), Some("failed"));
    let Status::Error { description } = &second.status else { panic!("not an error: {:?}", second.status) };
    assert!(description.contains("boom"), "{description}");
    assert_eq!(long_attr(second, "cicada.failures"), Some(1));
}

#[tokio::test(start_paused = true)]
async fn a_turn_that_fails_before_on_claim_still_gets_a_span() {
    let t = Telemetry::new().await;
    // State that does not decode: on_claim never runs, on_outcome does.
    let address = cicada::ActorAddress::parse("actor://traced/bad").unwrap();
    let template = cicada::UriTemplate::parse(Traced::TEMPLATE).unwrap();
    t.test.store().create(&address, &template, Some(json!("not a state"))).await.unwrap();
    t.test.send_raw(&address, "Hop", json!({ "left": 0 }), SendOptions::new()).await;
    t.test.run_until_idle().await;
    let spans = t.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(str_attr(&spans[0], "cicada.outcome").as_deref(), Some("failed"));
    assert!(matches!(spans[0].status, Status::Error { .. }));
}

#[tokio::test(start_paused = true)]
async fn turn_duration_and_lag_are_recorded_per_type_message_type_and_outcome() {
    let t = Telemetry::new().await;
    t.test.create(&traced(), &TracedState::default()).await;
    t.test.send(&traced(), TracedMsg::Hop { left: 1 }).await;
    t.test.run_until_idle().await;
    let duration = t.histogram("cicada.turn.duration");
    assert_eq!(duration.count(), 2);
    assert_eq!(attribute(duration.attributes(), "cicada.outcome"), Some("done".into()));
    assert_eq!(attribute(duration.attributes(), "cicada.actor.type"), Some("actor://traced/{id}".into()));
    assert_eq!(attribute(duration.attributes(), "cicada.message.type"), Some("Hop".into()));
    assert_eq!(t.histogram("cicada.message.lag").count(), 2);
}

#[tokio::test(start_paused = true)]
async fn trace_headers_hold_the_turns_traceparent_and_are_empty_without_the_plugin() {
    let t = Telemetry::new().await;
    t.test.create(&traced(), &TracedState::default()).await;
    t.test.send(&traced(), TracedMsg::Hop { left: 0 }).await;
    t.test.run_until_idle().await;
    let span = t.spans().pop().unwrap();
    let seen = t.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].keys().collect::<Vec<_>>(), vec!["traceparent"]);
    let expected = format!("00-{}-{}-", span.span_context.trace_id(), span.span_context.span_id());
    assert!(seen[0]["traceparent"].starts_with(&expected), "{seen:?}");

    let seen = Seen::default();
    let actor = TracedActor { seen: seen.clone() };
    let test = TestActors::builder().with(move |node| node.host(actor)).build().await.unwrap();
    test.create(&traced(), &TracedState::default()).await;
    test.send(&traced(), TracedMsg::Hop { left: 0 }).await;
    test.run_until_idle().await;
    assert_eq!(*seen.lock().unwrap(), vec![BTreeMap::new()]);
}

struct Stuck;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StuckRef {
    id: String,
}

impl ActorRef for StuckRef {
    const TEMPLATE: &'static str = "actor://stuck/{id}";
    type Message = StuckMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum StuckMsg {
    Wait,
}

impl Actor for Stuck {
    type Ref = StuckRef;
    type State = cicada::Stateless;
    type Message = StuckMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

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
    let plugin = ActorTelemetry::new(&tracer_provider, &meter_provider);
    let store = std::sync::Arc::new(cicada_test::InMemoryStore::default());
    let system = ActorSystem::builder(store)
        .host(Stuck)
        .install(plugin)
        .poll_interval(std::time::Duration::from_millis(10))
        .shutdown_cancel_timeout(std::time::Duration::from_secs(2))
        .build();
    system.start().await.unwrap();
    let stuck = StuckRef { id: "s".into() };
    system.create_stateless(&stuck).await.unwrap();
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
}
