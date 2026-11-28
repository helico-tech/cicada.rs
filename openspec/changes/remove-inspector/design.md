# Design

## Context

The owner ruled the inspector out of scope on 2026-11-28. `cicada-inspector` (ADR 0076) and the tray's `inspector`
feature are only in the unreleased 0.26.0: no published version has them, so they go without deprecation. The tray and
the terminal UI stay.

## Decisions

1. **Delete the crate and the tray's feature.** `crates/cicada-inspector` goes with its workspace member and
   dependency. `cicada-tray` loses the feature `inspector`, the "Open Inspector…" row (`InspectorItem`, `MenuModel`'s
   field `inspector`, the `close_inspector` parameter of `QuitRequest::begin`, and the tests
   `a_quit_disables_the_switch_the_contexts_quit_and_the_inspector` (it keeps its other checks),
   `open_inspector_is_feature_gated`, `the_inspector_uses_a_connection_in_any_phase_…` and
   `every_quit_request_closes_the_inspector_…` in `menu.rs`) and the AppKit code that opens and closes the window
   (`appkit.rs`: the `Embedded` field, its runtime handle, `trayOpenInspector:` and `close_inspector`; `appkit/view.rs`:
   the `inspector` menu item, its row, target and update). The menu keeps every other row in its order.
   Not taken: keeping the crate unpublished in the workspace (dead code the gate still builds).
2. **ADR 0078 supersedes ADR 0076.** ADR 0011 then holds again apart from ADR 0056's container layers; the tray keeps
   its own ADR 0074. Already written with this design (commit 1889dba8): ADR 0078, 0076's status, 0011's status and
   the index rows. No task.
3. **`ActorClient::changes` stays** (owner, R1a). ADR 0077 stands: the client gives Rust users the change hints that
   browsers have, and it is tested on its own (`crates/cicada-client/tests/changes.rs`). Not taken: removing it with
   ADR 0077 (less code, but rebuilt when a Rust user wants live updates).
4. **The unreleased inspector entries leave `CHANGELOG.md`** (owner, R2a): the crate never shipped, so a "removed" line
   would only confuse. The entries of `ActorClient::changes` and of the tray stay; sentences that name the inspector
   inside a tray entry or the summary lose that part (the summary's sentences on the inspector and `context_client`). Not taken: a "removed" line.
4b. **`cicada_app::context_client` goes** (owner, C1b, 2026-11-28: everything made for the inspector goes). It is
   unreleased and only the inspector calls it: the function, its re-export in `lib.rs`, its tests
   (`a_context_client_…`, `a_transport_error_of_a_context_client_…`) and its CHANGELOG entry go. `Contexts`,
   `ContextFile`, `ContextEntry` and `check_server_url` stay: the tray and the operator commands use them. Not taken:
   keeping it as a general helper.
5. **The gate drops the inspector.** `scripts/ci/feature-builds.sh` loses the check, clippy and tests with
   `--features inspector`; `scripts/ci/dependency-trees.sh` loses `cicada-inspector` from its list of application
   crates, the allowed tray edge and the `tray-inspector` tree with its two checks (lines 51-53). The header comments of
   both scripts stop naming the inspector. Its rule stays: no library crate has `cicada-containers` or `cicada-tray` in
   its tree.
6. **Specs.** The capability `inspector` is removed whole. The change-loop scenario "A screen with a board" names a
   tray screen instead of an inspector page; the rule itself stays for tray screens.
6b. **No tray board changes** (owner, B1a). Boards Main, Disconnected, Registered and Running show "Open Inspector…",
   but that row appeared only with the feature `inspector`; the default build already looked as it will now. No board
   is copied and no departure is listed.
7. **Archived changes stay** in `openspec/changes/archive/` as history; the link check skips that folder.

The local branch `feat/inspector-message-actions` was deleted on 2026-11-28 (owner, R3a). 0.26.0 still waits for the
owner's manual check of the tray menu and its Settings window (owner, R4a); that is a release step, not a task here.

## Docs this breaks

The docs session prepares, before landing (ADR 0045):

- `README.md` and `docs/README.md`: every mention of the inspector.
- `AGENTS.md`: the map row of `crates/cicada-inspector` and the mention in the tray row (`cicada-tray`'s feature).
- `docs/architecture.md`: the crate table's `cicada-inspector` row and the tray row's last sentence.
- `docs/guide/app.md`: the sentence on `cicada_app::context_client` in the config section.
- `docs/guide/app.md`: the mention of `cicada_inspector::Embedded` (near line 573).
- `docs/guide/app.md`: the section "Inspector on macOS" (anchor `#inspector-on-macos`), and the tray's paragraph
  "Open Inspector" with its command `--features cicada-tray/inspector`.
- `skills/cicada/SKILL.md`: `cicada-inspector` in the description's crate list and the route row to
  `docs/guide/app.md#inspector-on-macos`; `examples/tests/skill.rs` fails while the route points to a removed anchor.
