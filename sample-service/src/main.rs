//! The sample node: `cicada-sample-service serve` (or `worker`, or an operator command), configured by the flags and
//! variables of `cicada-app` and the few of its own (see `SampleConfig::from_vars`). SIGTERM or SIGINT stops a node in
//! good order: the node stops claiming and lets running turns finish within the grace period while the API still
//! answers, then the HTTP server stops, the store is closed and telemetry is flushed.

use std::process::ExitCode;

use cicada_sample_service::Sample;
use cicada_sample_service::app::run_process;

#[tokio::main]
async fn main() -> ExitCode {
    run_process(Sample::new).await
}
