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
   (newest first, the number of failed and abandoned ones as its badge), and while stopping the seconds left in the
   header's subtitle. Elapsed times are clamped at zero (the claim time is the store's clock, now is the local one). Its
   unit tests run in the gate everywhere. The `appkit` module (`cfg(target_os = "macos")`) only renders a `MenuModel`
   and sends actions back; it holds every `unsafe` call, each with its reason.
2. **`Tray::run(self) -> ExitCode` owns the main thread.** On macOS it starts a multi-threaded Tokio runtime on a
   background thread, enters its handle for the tray's spawns, creates the `NSApplication` (accessory policy, no Dock
   icon even without `LSUIElement`) and runs it. Elsewhere `run` returns at once with an error that the menu needs
   macOS, so a portable binary still compiles. Before macOS 14.0 `run` refuses with an error (exit code 1), checked at
   run time with `NSProcessInfo`, as a library cannot set the user's deployment target.
3. **Quit never blocks the main thread.** Quit (the item, ⌘Q) and Ctrl-C in a terminal (`tokio::signal::ctrl_c` on the
   runtime, which sets a flag the timer reads) start one quit: disconnect, then the timer waits until the tray holds no
   connection (`Offline` or `Failed`, as a worker that errors while stopping ends `Failed`); a second Quit while
   quitting changes nothing. With no connection the timer cancels the menu's tracking if it is open (`NSApp.stop` acts
   only after tracking ends), calls `NSApp.stop` and posts an empty event so the run loop returns. A system terminate
   (logout, restart) reaches `applicationShouldTerminate:`, which starts the same quit and answers `NSTerminateLater`;
   with no connection the timer replies `replyToApplicationShouldTerminate(true)` and AppKit ends the process (exit
   code 0, `run` does not return). Already without a connection, it answers `NSTerminateNow`. Until `worker-stop-now`
   a long grace period makes Quit and logout wait for it; the rustdoc says so.
   **Threads.** `run` builds the multi-threaded runtime on the main thread (its workers are background threads) and
   enters its handle for the tray's spawns; after the run loop returns it drops the tray, then the entered guard, then
   shuts the runtime down with `shutdown_timeout` of 5 seconds, and returns `ExitCode::SUCCESS`.
4. **No channel from the model to the view: a main-thread timer reads the watches.** An `NSTimer` every 250 ms, added
   to the common run-loop modes so it also fires while the menu tracks, checks `state()` and `turn_changes()` with
   `has_changed` and, while the menu is open (an `NSMenuDelegate` tracks `menuWillOpen`/`menuDidClose`) or once a
   second for the times, updates the menu from a new `MenuModel` in place: it changes the existing items' titles,
   states and badges and inserts or removes only the rows that changed, so the keyboard highlight and an open submenu
   stay. Menu actions call the tray's own methods (`connect`, `disconnect`) on the main thread; they only signal or
   spawn on the entered runtime, so they never block it. The timer and the actions target one Objective-C class made
   with `objc2`'s `define_class!` (no block-based timer, so no `block2`). Not taken: a channel and a dispatch to the
   main queue (one more crate, and the timer is needed anyway for the clock).
5. **The status item.** `NSStatusItem` with a template image (an SF Symbol through `NSImage`'s system symbol name,
   `isTemplate`), so it follows the menu bar's look; dimmed (`appearsDisabled`) when offline or failed, updated at the
   timer's 250 ms tick when the state changes.
6. **The menu's AppKit parts, macOS 14 or later.** The header is the only custom view (`NSView` with two labels and an
   `NSSwitch`); section headers are `NSMenuItem.sectionHeader(title:)`; the Recent Turns badge is `NSMenuItemBadge`;
   running turns use the item `subtitle`, which needs macOS 14.4: on 14.0 to 14.3 (checked at run time) the message
   type and attempt go into the title after the actor instead, so the proposal's macOS 14 stays the minimum. The
   builder confirms each API's version in `objc2-app-kit`'s availability attributes. Settings… (⌘,) is present and
   disabled until `tray-settings`; Quit (⌘Q).
7. **No Stop Now here.** Aborting the tray's task would not stop the turns: each turn and its lease renewal run in
   tasks of their own (design review). Cancelling them needs a core API, which the new change `worker-stop-now` adds;
   until then the menu shows the time left in the grace period and offers no Stop Now.
8. **What the model needs from `cicada-app`.** `WorkerReady` gains `types`, the names of the types the worker runs, for
   the checkmarks; it becomes `#[non_exhaustive]` and loses `Copy`. 0.25.0 ships `WorkerReady` before this change lands,
   so this is **BREAKING** for code that builds, copies or matches it without `..`: the change goes into 0.26.0. And
   `cicada_app::Contexts::load(&Env) -> anyhow::Result<Contexts>` with `names()` and `current()` reads the context names
   and the current context from the config file through the existing private config code (no tokens or servers leave
   it). The tray re-reads it each time the menu opens. `tray-settings` widens this later. 9. **Connect from the menu.**
   The tray starts `Offline`; the header switch connects to the current context (or, with none, the switch is disabled
   and the subtitle says to run `cicada config`), and off disconnects. Picking another context disconnects, waits until
   the tray holds no connection (`Offline` or `Failed`), and connects to it; the pending pick is kept in the main
   thread's state, which the timer acts on, and a later pick replaces it; picking the connected one does nothing. From
   `Failed` a pick connects at once. Not taken: connecting at launch (a setting for later). 10. **Dependencies.**
   `objc2`, `objc2-foundation` and `objc2-app-kit` (with only the features used), under `[target.'cfg(target_os =
   "macos")'.dependencies]`; `cargo deny check advisories` passes. A worked example `examples/menu-bar-worker.rs` (macOS
   only, `no_run` in the docs) shows `run`.

## Tests

`menu()` unit tests: each state's header; running turns with their elapsed time; the badge counting failed and
abandoned turns; the checkmarks from `WorkerReady::types`; the marked context; Stop Now with its seconds only while
stopping; no current context; a claim time after now shows zero. The quit and pending-pick logic as a plain state
machine with unit tests (quit while online, while stopping, twice, and from `Failed`; a stop that ends `Failed`; a
system terminate; a pick while stopping, replaced by a later pick; a pick from `Failed`; Quit clears a pending pick).
Model tests on virtual time: `WorkerReady::types` follows `--types`/`CICADA_TYPES`; `Contexts::load` reads names and the
current context and treats a missing file as none. The AppKit view gets a manual check list in the tasks (status item,
light and dark, keyboard, VoiceOver, the menu updating while open with a submenu open, Quit while a turn runs, logout,
Ctrl-C, Ctrl-C while the menu is open, macOS 14.0 without subtitles if a machine is at hand).

## Docs

The handoff gives the app guide a "Menu bar app on macOS" section (the `run` call, the deployment target, building a
`.app` with `LSUIElement`, signing as the user's step), and names the new example for the guide's list.
