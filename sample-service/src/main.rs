//! The sample node: `cicada-sample-service serve` (or `worker`, or an operator command), configured by the flags and
//! variables of `cicada-app` and the few of its own (see `SampleConfig::from_vars`). SIGTERM or SIGINT stops a node in
//! good order: the node stops claiming and lets running turns finish within the grace period while the API still
//! answers, then the HTTP server stops, the store is closed and telemetry is flushed.

use std::ffi::OsString;
use std::process::ExitCode;

use cicada_app::{Env, node_command};
use cicada_sample_service::telemetry::Telemetry;
use cicada_sample_service::{Sample, SampleConfig};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let env: Env =
        std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
    let Sample { app } = match Sample::new(&args, &env) {
        Ok(sample) => sample,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let wants_telemetry = SampleConfig::from_vars(&env).is_ok_and(|c| c.telemetry);
    let telemetry = if node_command(&args) && wants_telemetry {
        match Telemetry::from_env() {
            Ok(telemetry) => Some(telemetry),
            Err(e) => {
                eprintln!("error: OpenTelemetry: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let app = match &telemetry {
        Some(t) => app.observe(t.observer()),
        None => app,
    };
    let code = app.run_async().await;
    if let Some(t) = telemetry {
        t.shutdown().await;
    }
    code
}
