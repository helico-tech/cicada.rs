//! OTLP export of traces and metrics, when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
//!
//! The exporters speak OTLP over HTTP with protobuf (`http/protobuf`, port 4318) and read the standard variables of the
//! OpenTelemetry SDK for Rust: `OTEL_EXPORTER_OTLP_ENDPOINT` (and the per-signal `…_TRACES_ENDPOINT`,
//! `…_METRICS_ENDPOINT`), `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_EXPORTER_OTLP_TIMEOUT`, `OTEL_SERVICE_NAME`,
//! `OTEL_RESOURCE_ATTRIBUTES`, `OTEL_BSP_*` (span batching) and `OTEL_METRIC_EXPORT_INTERVAL`. gRPC is not built in:
//! `OTEL_EXPORTER_OTLP_PROTOCOL=grpc` is refused. Trace context travels in W3C `traceparent`/`tracestate` headers.

use cicada_otel::ActorTelemetry;
use opentelemetry::propagation::TextMapCompositePropagator;
use opentelemetry_otlp::{MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
use opentelemetry_sdk::trace::SdkTracerProvider;

/// The SDK's tracer and meter providers, exporting over OTLP.
#[derive(Debug)]
pub struct Telemetry {
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
}

impl Telemetry {
    /// Builds the exporters and providers from the `OTEL_*` variables and installs the W3C trace-context and baggage
    /// propagators globally.
    pub fn from_env() -> anyhow::Result<Self> {
        if let Ok(protocol) = std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL")
            && !protocol.trim().is_empty()
            && protocol.trim() != "http/protobuf"
        {
            anyhow::bail!("OTEL_EXPORTER_OTLP_PROTOCOL: only http/protobuf is supported, was {protocol}");
        }
        let resource = Resource::builder().build();
        let spans = SpanExporter::builder().with_http().with_protocol(Protocol::HttpBinary).build()?;
        let metrics = MetricExporter::builder().with_http().with_protocol(Protocol::HttpBinary).build()?;
        let tracer_provider =
            SdkTracerProvider::builder().with_resource(resource.clone()).with_batch_exporter(spans).build();
        let meter_provider =
            SdkMeterProvider::builder().with_resource(resource).with_periodic_exporter(metrics).build();
        opentelemetry::global::set_text_map_propagator(TextMapCompositePropagator::new(vec![
            Box::new(TraceContextPropagator::new()),
            Box::new(BaggagePropagator::new()),
        ]));
        Ok(Self { tracer_provider, meter_provider })
    }

    /// The node plugin that records turns through these providers.
    pub fn plugin(&self) -> ActorTelemetry {
        ActorTelemetry::new(&self.tracer_provider, &self.meter_provider)
    }

    /// Flushes what is buffered and shuts the providers down. Blocks while exporting, so it runs on a blocking thread.
    pub async fn shutdown(self) {
        let ended = tokio::task::spawn_blocking(move || {
            if let Err(e) = self.tracer_provider.shutdown() {
                tracing::warn!("Shutting down the tracer provider failed: {e}");
            }
            if let Err(e) = self.meter_provider.shutdown() {
                tracing::warn!("Shutting down the meter provider failed: {e}");
            }
        })
        .await;
        if let Err(e) = ended {
            tracing::warn!("Shutting down telemetry failed: {e}");
        }
    }
}
