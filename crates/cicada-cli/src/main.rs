//! The `cicada` binary.

use std::process::ExitCode;

use cicada_cli::cli::Cli;
use clap::Parser;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let env = std::env::vars().collect();
    let mut out = std::io::stdout().lock();
    match cicada_cli::run(cli, &env, &mut out).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", cicada_cli::error_text(&e));
            ExitCode::FAILURE
        }
    }
}
