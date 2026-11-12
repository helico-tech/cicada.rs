//! OpenTelemetry traces and metrics for Cicada turns: the [`ActorTelemetry`] observer.
//!
//! Uses the OpenTelemetry API only; the service brings the SDK and its exporters.
//!
//! **Traces.** One `CONSUMER` span per turn, named `process <type name>`, from the claim to the commit, with
//! `messaging.system` = `cicada`, `messaging.operation.name` = `process`, `messaging.destination.name` (the type name),
//! `messaging.message.id`, `messaging.message.conversation_id` (the correlation id), and `cicada.actor`,
//! `cicada.message.type`, `cicada.attempt`, `cicada.failures`, `cicada.outcome` (`done`, `failed`, `rescheduled`),
//! `cicada.sent` (messages the commit enqueued). A turn that ends `failed` has status `ERROR` with the error, and a turn
//! that ends `failed` or is rescheduled has an `exception` event with the error: a failure of the handler, a timeout, a
//! message or a state that cannot be decoded, or a rejected commit. Its parent is the trace context in the message's
//! headers (W3C `traceparent`/`tracestate`, or whatever the configured propagator reads). Every message a turn sends
//! carries the turn's trace context, so the turns of a chain of messages form one trace without any code in the
//! handlers; code outside an actor puts its context on a send with [`current_trace_headers`]. A turn of which the node
//! saw no commit succeed (its lease was lost, its node stopped mid-turn, its commit did not get through) has no
//! outcome; the OpenTelemetry SDK for Rust exports every span it drops, so its span is ended when the node drops the
//! turn, with `cicada.outcome` `abandoned`, status unset and no `exception` event, also when its handler failed. The
//! re-run gets its own span.
//!
//! **Metrics.** `cicada.turn.duration` (histogram, seconds, claim to commit) and `cicada.message.lag` (histogram,
//! seconds, from the message's due time to its claim), both with `cicada.actor.type` (the type name) and
//! `cicada.message.type`; the duration also with `cicada.outcome`. Its count is the number of turns per outcome.
//!
//! ```
//! # fn node(store: cicada::Store) {
//! use cicada::ActorSystem;
//! use cicada_otel::ActorTelemetry;
//!
//! // The global tracer, meter and propagator; or ActorTelemetry::new(&tracer_provider, &meter_provider).
//! let system = ActorSystem::builder(store).observe(ActorTelemetry::global()).build();
//! # }
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use cicada::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
use cicada::store::TurnOutcome;
use opentelemetry::global::{self, BoxedTracer, ObjectSafeTracerProvider};
use opentelemetry::metrics::{Histogram, Meter, MeterProvider};
use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry::trace::{SpanKind, Status, TraceContextExt, Tracer};
use opentelemetry::{Context, InstrumentationScope, KeyValue};

/// The instrumentation scope name of the tracer and meter.
pub const INSTRUMENTATION_NAME: &str = "nl.helico.cicada";

const MESSAGING_SYSTEM: &str = "messaging.system";
const MESSAGING_OPERATION: &str = "messaging.operation.name";
const MESSAGING_DESTINATION: &str = "messaging.destination.name";
const MESSAGING_MESSAGE_ID: &str = "messaging.message.id";
const MESSAGING_CONVERSATION_ID: &str = "messaging.message.conversation_id";
const ACTOR: &str = "cicada.actor";
const ACTOR_TYPE: &str = "cicada.actor.type";
const MESSAGE_TYPE: &str = "cicada.message.type";
const ATTEMPT: &str = "cicada.attempt";
const FAILURES: &str = "cicada.failures";
const OUTCOME: &str = "cicada.outcome";
const SENT: &str = "cicada.sent";

/// OpenTelemetry traces and metrics for turns, an observer of a node; see the [crate] documentation.
pub struct ActorTelemetry {
    tracer: BoxedTracer,
    duration: Histogram<f64>,
    lag: Histogram<f64>,
    /// `None`: the global propagator.
    propagator: Option<Arc<dyn TextMapPropagator + Send + Sync>>,
}

impl ActorTelemetry {
    /// Telemetry through the global tracer provider, meter provider and text map propagator, which are no-ops until
    /// the service installs an SDK.
    pub fn global() -> Self {
        let scope = InstrumentationScope::builder(INSTRUMENTATION_NAME).build();
        Self::build(global::tracer_with_scope(scope.clone()), &global::meter_with_scope(scope))
    }

    /// Telemetry through the given providers. Trace headers are read and written by the global propagator unless
    /// [`Self::with_propagator`] sets one.
    pub fn new(tracer_provider: &impl ObjectSafeTracerProvider, meter_provider: &impl MeterProvider) -> Self {
        let scope = InstrumentationScope::builder(INSTRUMENTATION_NAME).build();
        let tracer = BoxedTracer::new(tracer_provider.boxed_tracer(scope.clone()));
        Self::build(tracer, &meter_provider.meter_with_scope(scope))
    }

    /// Reads and writes trace headers with `propagator` (such as the SDK's `TraceContextPropagator`) instead of the
    /// global one.
    pub fn with_propagator(mut self, propagator: impl TextMapPropagator + Send + Sync + 'static) -> Self {
        self.propagator = Some(Arc::new(propagator));
        self
    }

    fn build(tracer: BoxedTracer, meter: &Meter) -> Self {
        let duration = meter
            .f64_histogram("cicada.turn.duration")
            .with_unit("s")
            .with_description("Turns, from the claim to the commit")
            .build();
        let lag = meter
            .f64_histogram("cicada.message.lag")
            .with_unit("s")
            .with_description("How late a message was claimed after it was due")
            .build();
        Self { tracer, duration, lag, propagator: None }
    }

    fn with_propagator_do<T>(&self, mut f: impl FnMut(&dyn TextMapPropagator) -> T) -> T {
        match &self.propagator {
            Some(p) => f(p.as_ref()),
            None => global::get_text_map_propagator(|p| f(p)),
        }
    }

    fn inject(&self, cx: &Context) -> BTreeMap<String, String> {
        let mut headers = Headers::default();
        self.with_propagator_do(|p| p.inject_context(cx, &mut headers));
        headers.0
    }

    /// The span of the turn that starts, as the child of the trace context in its message's headers.
    fn start_span(&self, turn: &TurnStart<'_>) -> Context {
        let message = turn.message();
        let parent =
            self.with_propagator_do(|p| p.extract_with_context(&Context::new(), &HeaderReader(&message.headers)));
        let type_name = turn.type_name().as_str().to_owned();
        let mut attributes = vec![
            KeyValue::new(MESSAGING_SYSTEM, "cicada"),
            KeyValue::new(MESSAGING_OPERATION, "process"),
            KeyValue::new(MESSAGING_DESTINATION, type_name.clone()),
            KeyValue::new(MESSAGING_MESSAGE_ID, message.id.to_string()),
            KeyValue::new(ACTOR, turn.actor().to_string()),
            KeyValue::new(MESSAGE_TYPE, message.message_type.clone()),
            KeyValue::new(ATTEMPT, i64::from(message.attempt)),
            KeyValue::new(FAILURES, i64::from(message.failures)),
        ];
        if let Some(correlation_id) = &message.correlation_id {
            attributes.push(KeyValue::new(MESSAGING_CONVERSATION_ID, correlation_id.clone()));
        }
        let span = self
            .tracer
            .span_builder(format!("process {type_name}"))
            .with_kind(SpanKind::Consumer)
            .with_attributes(attributes)
            .start_with_context(&self.tracer, &parent);
        Context::new().with_span(span)
    }
}

impl Default for ActorTelemetry {
    /// [`ActorTelemetry::global`].
    fn default() -> Self {
        Self::global()
    }
}

impl TurnObserver for ActorTelemetry {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        let cx = self.start_span(turn);
        // The messages this turn sends are children of its span.
        for (name, value) in self.inject(&cx) {
            turn.add_send_header(name, value);
        }
        let message = turn.message();
        let (type_name, message_type) = (turn.type_name().as_str().to_owned(), message.message_type.clone());
        let lag = (turn.claimed_at() - message.deliver_at).to_std().unwrap_or_default();
        self.lag.record(
            lag.as_secs_f64(),
            &[KeyValue::new(ACTOR_TYPE, type_name.clone()), KeyValue::new(MESSAGE_TYPE, message_type.clone())],
        );
        Box::new(TurnSpan {
            cx,
            started: Instant::now(),
            duration: self.duration.clone(),
            type_name,
            message_type,
            ended: false,
        })
    }
}

/// The trace headers for the caller's current OpenTelemetry context ([`Context::current`]), written by the global
/// propagator, for sends from outside an actor: `system.send_with(&to, msg, SendOptions { headers:
/// current_trace_headers(), ..Default::default() })`, or with the HTTP client.
pub fn current_trace_headers() -> BTreeMap<String, String> {
    let mut headers = Headers::default();
    global::get_text_map_propagator(|p| p.inject_context(&Context::current(), &mut headers));
    headers.0
}

/// A turn's span (in its context), when it started and what its duration is recorded with. Dropped without an end, it
/// ends the span with `cicada.outcome = abandoned`: the SDK exports every span it drops, so it is exported and marked.
struct TurnSpan {
    cx: Context,
    started: Instant,
    duration: Histogram<f64>,
    type_name: String,
    message_type: String,
    ended: bool,
}

impl ObservedTurn for TurnSpan {
    fn turn_ended(mut self: Box<Self>, end: &TurnEnd<'_>) {
        let span = self.cx.span();
        let exception = |error: &str| {
            span.add_event("exception", vec![KeyValue::new("exception.message", error.to_owned())]);
        };
        let name = match end.outcome() {
            TurnOutcome::Done => "done",
            TurnOutcome::Failed(error) => {
                exception(error);
                span.set_status(Status::error(error.clone()));
                "failed"
            }
            TurnOutcome::Rescheduled { error, .. } => {
                exception(error);
                "rescheduled"
            }
            TurnOutcome::Cancelled => "cancelled",
        };
        span.set_attribute(KeyValue::new(OUTCOME, name));
        span.set_attribute(KeyValue::new(SENT, end.sent().len() as i64));
        span.end();
        self.duration.record(
            self.started.elapsed().as_secs_f64(),
            &[
                KeyValue::new(ACTOR_TYPE, self.type_name.clone()),
                KeyValue::new(MESSAGE_TYPE, self.message_type.clone()),
                KeyValue::new(OUTCOME, name),
            ],
        );
        self.ended = true;
    }
}

impl Drop for TurnSpan {
    fn drop(&mut self) {
        if !self.ended {
            let span = self.cx.span();
            span.set_attribute(KeyValue::new(OUTCOME, "abandoned"));
            span.end();
        }
    }
}

/// Reads message headers; names are case-insensitive on the wire, and senders may well write `Traceparent`.
struct HeaderReader<'a>(&'a BTreeMap<String, String>);

impl Extractor for HeaderReader<'_> {
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

/// Collects headers written by a propagator. Empty values are left out: the Rust W3C propagator writes an empty
/// `tracestate`, which means the same as none.
#[derive(Default)]
struct Headers(BTreeMap<String, String>);

impl Injector for Headers {
    fn set(&mut self, key: &str, value: String) {
        if !value.is_empty() {
            self.0.insert(key.to_owned(), value);
        }
    }
}
