//! The sample without bootstrap: `cicada-sample-executor worker --server <url> --types <names>` runs the sample's
//! types over HTTP, which `cicada-sample-service` cannot, as its singletons and dead-letter watch need `App::on_start`.
//! It has the whole command line of `cicada-app` (`serve`, `worker` on a store, the operator commands), and none of
//! its modes creates singletons or watches. SIGTERM or SIGINT stops it in good order.

use std::process::ExitCode;

use cicada_sample_service::Sample;
use cicada_sample_service::app::run_process;

#[tokio::main]
async fn main() -> ExitCode {
    run_process(Sample::executor).await
}
