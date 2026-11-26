# Design

`tray-worker-control` gave the tray a model with no GUI. This change adds the macOS menu bar app on top of it, native
AppKit as the owner chose, laid out as in the mockup: a status item, and a menu with a header switch, the running turns,
the contexts, the registered types and the recent turns.

```rust
fn main() -> std::process::ExitCode {
    cicada_tray::Tray::new(|| App::new().host(runner_type())).run()   // owns the main thread; macOS only
}
```

## Decisions

1. **A portable menu model, a thin AppKit view.** A plain function `menu(&MenuInput) -> MenuModel` (module `menu`,
   every platform) turns the state, the turn snapshot, the registered types, the worker's types, the contexts and the
   current time into the menu's items: the header (title, subtitle, switch on or off, enabled), the Running section
   (actor, message type, attempt, time since the claim, rounded to seconds), the Context section (names, the marked
   one), the Registered Types submenu (a checkmark for each type the connected worker runs), the Recent Turns submenu
   (newest first, the number of failed and abandoned ones as its badge), and the Stop Now item while stopping, with
   the seconds left. Its unit tests run in the gate everywhere. The `appkit` module (`cfg(target_os = "macos")`) only
   renders a `MenuModel` and sends actions back; it holds every `unsafe` call, each with its reason.
2. **`Tray::run(self) -> ExitCode` owns the main thread.** On macOS it starts a multi-threaded Tokio runtime on a
   background thread, enters its handle for the tray's spawns, creates the `NSApplication` (accessory policy, no Dock
   icon even without `LSUIElement`) and runs it. Elsewhere `run` returns at once with an error that the menu needs
   macOS, so a portable binary still compiles. Quit disconnects, waits for `Offline` up to the stop's `until`, then
   ends the run loop.
3. **No channel from the model to the view: a main-thread timer reads the watches.** An `NSTimer` every 250 ms, added
   to the common run-loop modes so it also fires while the menu tracks, checks `state()` and `turn_changes()` with
   `has_changed` and, while the menu is open (an `NSMenuDelegate` tracks `menuWillOpen`/`menuDidClose`) or once a
   second for the times, rebuilds the menu's items from a new `MenuModel` in place. Menu actions call the tray's own
   methods (`connect`, `disconnect`, `stop_now`) on the main thread; they only signal or spawn on the entered runtime,
   so they never block it. Not taken: a channel and a dispatch to the main queue (one more crate, and the timer is
   needed anyway for the clock).
4. **The status item.** `NSStatusItem` with a template image (an SF Symbol through `NSImage`'s system symbol name,
   `isTemplate`), so it follows the menu bar's look; dimmed (`appearsDisabled`) when offline or failed.
5. **The menu's AppKit parts, macOS 14 or later.** The header is the only custom view (`NSView` with two labels and an
   `NSSwitch`); section headers are `NSMenuItem.sectionHeader(title:)`; the Recent Turns badge is `NSMenuItemBadge`;
   running turns use the item `subtitle`. `subtitle` needs macOS 14.4, so the deployment target is 14.4 (the
   proposal's 14.0 moves up by this one API; the builder confirms each API's version in Apple's headers through
   `objc2-app-kit`'s availability attributes). Settings… (⌘,) is present and disabled until `tray-settings`; Quit
   (⌘Q).
6. **Stop Now.** `Tray::stop_now()`, valid while `Stopping`, aborts the worker's task: its running turns end as
   `abandoned` at once (the observer's `Drop`), and the server takes their claims back when their leases expire, as
   when a process is killed. The task guard ends in `Offline`, not `Failed`, for an abort the tray asked for. The menu
   offers it only while stopping.
7. **What the model needs from `cicada-app`.** `WorkerReady` (new in 0.25.0, so not breaking) gains `types`, the names
   of the types the worker runs, for the checkmarks; it becomes `#[non_exhaustive]` and loses `Copy`. And
   `cicada_app::Contexts::load(&Env) -> anyhow::Result<Contexts>` with `names()` and `current()` reads the context names
   and the current context from the config file through the existing private config code (no tokens or servers
   leave it). The tray re-reads it each time the menu opens. `tray-settings` widens this later.
8. **Connect from the menu.** The tray starts `Offline`; the header switch connects to the current context (or, with
   none, the switch is disabled and the subtitle says to run `cicada config`), and off disconnects. Picking another
   context disconnects, waits for `Offline`, and connects to it; picking the connected one does nothing. Not taken:
   connecting at launch (a setting for later).
9. **Dependencies.** `objc2`, `objc2-foundation` and `objc2-app-kit` (with only the features used), under
   `[target.'cfg(target_os = "macos")'.dependencies]`; `cargo deny check advisories` passes. A worked example
   `examples/menu-bar-worker.rs` (macOS only, `no_run` in the docs) shows `run`.

## Tests

`menu()` unit tests: each state's header; running turns with their elapsed time; the badge counting failed and
abandoned turns; the checkmarks from `WorkerReady::types`; the marked context; Stop Now with its seconds only while
stopping; no current context. Model tests on virtual time: `stop_now` while stopping ends `Offline` with the running
turn `abandoned`; `stop_now` in another state does nothing; `WorkerReady::types` follows `--types`/`CICADA_TYPES`;
`Contexts::load` reads names and the current context and treats a missing file as none. The AppKit view gets a manual
check list in the tasks (status item, light and dark, keyboard, VoiceOver, the menu updating while open, Quit).

## Docs

The handoff gives the app guide a "Menu bar app on macOS" section (the `run` call, the deployment target, building a
`.app` with `LSUIElement`, signing as the user's step), and names the new example for the guide's list.
