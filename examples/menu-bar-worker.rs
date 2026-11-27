//! Run a macOS menu bar worker with `cargo run -p cicada-examples --example menu-bar-worker`.
//!
//! Use `cicada config set-context` and `cicada config use-context` to select a node first, then use the menu bar
//! switch to connect. The worker runs the checkout actor from the timer example.

use std::process::ExitCode;

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    use cicada_app::App;
    use cicada_examples::timer::CheckoutActor;
    use cicada_tray::Tray;

    Tray::new(|| App::new().host(CheckoutActor)).run()
}

#[cfg(not(target_os = "macos"))]
fn main() -> ExitCode {
    eprintln!("the menu-bar-worker example requires macOS 14 or later");
    ExitCode::FAILURE
}
