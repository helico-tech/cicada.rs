//! The `cicada` binary: the operator commands, with no hosted type.

use std::process::ExitCode;

fn main() -> ExitCode {
    cicada_app::App::new().version(concat!("cicada ", env!("CARGO_PKG_VERSION"))).run()
}
