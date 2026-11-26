Verdict: fix needed

## Blocking

1. **Two spec scenarios have no test.** These are "A running turn before macOS 14.4" and "An old macOS" in `specs/menu-bar-tray/spec.md`.
   - The 14.4 fallback is built inside the view closure (`crates/cicada-tray/src/appkit/view.rs:128-137`). The text is
     `"{actor}     {elapsed}s"` plus `"{type} · Attempt {n}"`, or both joined in the title. No portable code holds it, so
     no test can reach it.
   - The 14.0 refusal (`appkit.rs:44-46`) is untested as well.
   - What fixed looks like:
     - Move the row text into `menu` as a pure function of `RunningRow` and `subtitles: bool`, returning a title and
       an optional subtitle. Decision 1 says the view only renders. Test both branches.
     - Give the version refusal a small pure function that returns the error and exit code. Test it.

## Notes

1. Task 3 is unticked: the manual GUI checks are still open (the handoff says so). The owner must run them before
   landing. Most at risk: Ctrl-C while the menu is open (`cancelTracking`, then `stop` and the posted event), and
   keyboard Return on the header item, whose custom view may swallow the action.
2. `crates/cicada-tray/Cargo.toml` stops using `[lints] workspace = true` and lowers `unsafe_code` from forbid to deny
   for the whole crate. This is needed for the module-level allow. But future workspace lints will not reach this
   crate. Add a comment saying the copy must follow `[workspace.lints]`.
3. `connect` now runs the factory through `spawn_blocking`. Its "the App factory panicked" path has no test, here or
   before.
4. Rebase onto main (0.25.0 is released). `CHANGELOG.md` will conflict: keep `## 0.26.0 — unreleased` above the dated
   0.25.0 section.
5. Once tokio owns SIGINT, a second Ctrl-C cannot force-quit during a long grace period. This matches the design and
   the rustdoc, but the guide should say it.

## Checked and fine

- Every `unsafe` is in `appkit` and has a SAFETY reason.
- Main thread: `MainThreadMarker` is used and the class is `MainThreadOnly`.
- The timer runs in the common modes.
- Rows are synced in place (`Rows::sync` is correct for inserts, removes and reorders).
- Quit and pending-pick logic, including the terminate reply.
- Shutdown order: tray, then guard, then `shutdown_timeout(5s)`.
- `WorkerReady` is non_exhaustive, no longer Copy, and its types follow host order.
- `Contexts::load` leaks no tokens or servers (the TOML error is cut down to its context, and there is a test).
- Dependencies are macOS-only.
- Versions are 0.26.0 everywhere, and the CHANGELOG has BREAKING.
- No protected docs are edited, and the handoff matches the code.
- `cargo test -p cicada-tray`: 23 tests pass.

## Not checked

- The non-macOS build: `cargo check --target aarch64-linux-android` failed on a missing C toolchain. I read the
  `cfg`s by hand and they look right.
- The GUI behaviour, and the workspace tests.
