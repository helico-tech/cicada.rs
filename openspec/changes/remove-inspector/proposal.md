# Proposal

## Why

The owner ruled the inspector out of scope on 2026-11-28: "Remove everything for the inspector". The crate
`cicada-inspector` (ADR 0076) and the tray's optional `inspector` feature exist only in the unreleased 0.26.0; no
published version has them. Removing them before 0.26.0 ships costs no deprecation and no upgrade notes. The same day
the owner dropped the open inspector changes (`inspector-message-actions` and the mockup fixes).

## What Changes

- Delete `crates/cicada-inspector` and its workspace entries (`Cargo.toml` member and dependency, `Cargo.lock`).
- `cicada-tray` loses its `inspector` feature, the "Open Inspector…" menu row (`InspectorItem`, `MenuModel`'s
  `inspector` field) and the AppKit code that opens and closes the window. The tray itself stays.
- The gate's checks drop the inspector: `scripts/ci/feature-builds.sh` (check, clippy and tests with `inspector`) and
  `scripts/ci/dependency-trees.sh` (the allowed tray-to-inspector edge).
- A new ADR supersedes ADR 0076, so ADR 0011 ("library only") holds again in full, apart from the tray (ADR 0074).
- The `inspector` capability spec is removed. The change-loop spec's GUI scenario stops naming an inspector page.
- `CHANGELOG.md`: the unreleased inspector entries go, as the crate never shipped.

To decide in the design: whether `ActorClient::changes` (ADR 0077, unreleased, written for the inspector, now without
a consumer in the repo) stays; whether the CHANGELOG entries go or a "removed" line is written; what happens to the
local branch `feat/inspector-message-actions`.

## Out of scope

- The tray (`cicada-tray`): its menu, Settings window and worker control stay.
- The terminal UI of `cicada-app` (feature `tui`).
- The archived inspector changes in `openspec/changes/archive/`: history, kept as is.

## Order

After `tray-menu-mockup`. Before `worker-view`.

## Impact

Crates: cicada-inspector, cicada-tray.

Workspace `Cargo.toml` and `Cargo.lock`, `scripts/ci/feature-builds.sh`, `scripts/ci/dependency-trees.sh`, a new ADR
with its row in `docs/adr/README.md`, `openspec/specs/inspector` and `openspec/specs/change-loop` deltas,
`CHANGELOG.md`. Docs handoff: README, AGENTS.md (the map row of `cicada-inspector` and the tray row's mention),
`docs/README.md`, `docs/architecture.md` (the crate row and the tray row), `docs/guide/app.md` (section "Inspector on
macOS"), `skills/cicada/SKILL.md` (the crate list in its description and the route to that section), and the mention
in ADR 0073's context stays (an accepted ADR is not rewritten).
