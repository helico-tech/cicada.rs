# Docs handoff for tray-menu-bar

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada_tray::Tray::run` drives a native menu bar app. It starts offline and connects an in-process HTTP worker
through the config file. `Contexts::load(&Env)` reads the configured names in alphabetical order and the file's
current selection. `App::on_worker_ready` supplies the selected `WorkerReady::types` in host registration order.
`WorkerReady` is now non-exhaustive and no longer implements `Copy`. The app tests cover flag and environment type
selection and context loading. The tray's `menu::tests` cover the header, elapsed time, recent turns, checkmarks,
context switching and both quit paths. The worked example is `examples/menu-bar-worker.rs`.

## 1. Menu bar app on macOS

**Where.** `docs/guide/app.md`, `Menu bar app on macOS`, and the guide's worked examples list.
**Text.** Use a plain `main` that returns `std::process::ExitCode` and calls `Tray::new(|| App::new().host(MyActor)).run()`.
The runnable source in `examples/menu-bar-worker.rs` hosts the checkout actor. If included as a Rust block, use
`no_run` and gate the macOS code. Run it with `cargo run -p cicada-examples --example menu-bar-worker`.
Configure and select a context with the Cicada CLI before opening the menu. The switch connects to the file's
current context and disconnects gracefully. Picking a different context drains the current worker before starting
another. The menu reloads the context names each time it opens. Running shows the actor, message type, attempt and
time since claim. Registered Types marks the types the worker runs, and Recent Turns shows the most recent 50
observed finishes with a badge for failures and abandoned turns. Quit and Ctrl-C drain the worker before the app
exits. The same drain runs during logout. Settings is disabled.

**Caveats.** Run on the main thread on macOS 14 or later. On 14.0–14.3, the running turn's details go in its title.
For a distributed `.app`, build the binary with the intended `MACOSX_DEPLOYMENT_TARGET`, place it in
`Contents/MacOS`, put its executable and bundle metadata in `Contents/Info.plist`, and include `LSUIElement` set to
true for a menu bar agent. The running app uses the accessory policy so it has no Dock icon. Signing and notarizing
the bundle belong to the host's build. The app factory is called at launch to list registered types and for each
accepted connection. The worker's grace period and cancel timeout can delay Quit and logout.

## 2. Reading the selected worker types

**Where.** `docs/guide/app.md`, the HTTP worker section, and `docs/how-to/upgrading.md`, the 0.26.0 upgrade.
**Text.** Read `WorkerReady::types` inside `App::on_worker_ready` to get the types selected by `--types` or
`CICADA_TYPES`. Destructure the new non-exhaustive type with `..` and call `clone()` when a second owned value is
needed. Remove constructions of `WorkerReady` outside `cicada-app` and obtain it from the callback.
**Caveats.** These names are for the current HTTP worker. `App::type_names` lists every type the factory hosts.

## Blocking

None.

## Status

Proposed guide and upgrade prose. App and tray tests, the tray build and Clippy, and the advisory check passed.
The docs checks passed (11 link and guarantee tests and 51 doctests). The OpenAPI version was regenerated.
The example started from a terminal and exited successfully on Ctrl-C. A temporary bundle with `LSUIElement`
also started and shut down on SIGINT. The GUI automation session could not observe the status menu, so the
remaining manual checks in task 3 are open.
