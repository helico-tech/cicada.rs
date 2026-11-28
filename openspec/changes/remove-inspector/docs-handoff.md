# Docs handoff for remove-inspector

Version: 0.26.0 (unreleased). Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The inspector is gone before it shipped (ADR 0078 supersedes ADR 0076). No published version had it, so there is no
deprecation and no upgrade note.

- The crate `cicada-inspector` is deleted, with its workspace member and dependency.
- `cicada-tray` has no feature `inspector` and no "Open Inspector…" menu row. The menu keeps every other row in the same
  order; the default build looks as before.
- `cicada_app::context_client` is removed. `Contexts`, `ContextFile`, `ContextEntry` and `check_server_url` stay.
- `ActorClient::changes` stays (ADR 0077).
- The unreleased inspector and `context_client` entries left `CHANGELOG.md`.

No guarantee row, wire format, schema, command line or actor contract changes. Every item below removes prose; none adds
any.

## 1. README: the crate row

**Where.** `README.md`, the table of crates and guides (the row "A native inspector window for macOS").
**Text.** Remove the row `A native inspector window for macOS | cicada-inspector | Inspector on macOS`.
**Caveats.** Its link points to `docs/guide/app.md#inspector-on-macos`, which item 5 removes.

## 2. docs/README.md: the task row

**Where.** `docs/README.md`, the row "Inspect a running system from a Mac".
**Text.** Remove the row; it links to the removed section.

## 3. AGENTS.md: the map

**Where.** `AGENTS.md`, `Map`.
**Text.** Remove the row of `crates/cicada-inspector`. In the row of `crates/cicada-tray`, drop any mention of the
inspector or of the tray's feature `inspector`.

## 4. docs/architecture.md: the crate table

**Where.** `docs/architecture.md`, the crate table.
**Text.** Remove the `cicada-inspector` row. In the `cicada-tray` row, end after "…tracks running turns and the latest
50 finished turns. On macOS, `Tray::run` drives a native menu bar app (AppKit)", without "; its optional feature
`inspector` adds Open Inspector…, which opens `cicada-inspector`'s `Embedded` window in the tray's process".

## 5. docs/guide/app.md: the tray paragraph, the inspector section and context_client

**Where.** `docs/guide/app.md`:
- `Menu bar app on macOS`, the paragraph that starts "**Open Inspector.**" (with its command
  `cargo run -p cicada-examples --example menu-bar-worker --features cicada-tray/inspector`).
- `Inspector on macOS` (anchor `#inspector-on-macos`) with all its text and its subsection `Live updates` (anchor
  `#live-updates`), up to `Command line and terminal UI`. This includes the bullet on `cicada_inspector::Embedded`.
- `Server and credentials`: the sentences from "`cicada_app::context_client(&env, Some("prod"))` (from 0.26.0) gives an
  `ActorClient` for a context…" to "…the client's URL keeps neither."
**Text.** Remove them. The paragraph in `Server and credentials` then reads "Context headers/token still apply when its
URL is overridden. A missing file is empty; an unreadable file or unknown context fails. `config set-context` replaces
a context; …".
**Caveats.** No other file links to `#live-updates`; check the guide's own links after the cut.

## 6. skills/cicada/SKILL.md: the crate list and the route

**Where.** `skills/cicada/SKILL.md`: the `description` front matter, and the route table row "An inspector window on
macOS".
**Text.** Drop `cicada-inspector` from the description's crate list, which then ends "…, cicada-containers,
cicada-tray)". Remove the route row `An inspector window on macOS | docs/guide/app.md#inspector-on-macos,
crates/cicada-inspector`.
**Caveats.** `metadata.cicada-version` stays 0.26.0.

## Not changed

- ADR 0073's context names the inspector; an accepted ADR is not rewritten.
- ADRs 0011, 0076 and 0078 and the index rows were written with the design.
- `.claude/skills/change-loop/SKILL.md`, "GUI changes": `main` already dropped "an inspector page" (commit `161c731f`).

## Blocking

Integrated. The skill route check (`cargo test -p cicada-examples --test skill`,
`every_route_resolves_to_a_file_and_heading_of_this_source`) failed while `skills/cicada/SKILL.md` routed to
`crates/cicada-inspector`. The owner prepared items 1 to 6 in commit `cf6f5079` of the branch `docs`: `README.md`,
`docs/README.md`, `AGENTS.md`, `docs/architecture.md`, `docs/guide/app.md` and `skills/cicada/SKILL.md`, taken as they
are and folded into the commit that deletes the crate.

## Status

Ran on the builder's head, with the owner's fix: `cargo test -p cicada-examples --test skill` (18 passed), `--test
docs` (11 passed) and `--doc` (55 passed), `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
warnings` (clean), `cargo test --workspace --no-fail-fast` (3891 passed, 0 failed), `bash
scripts/ci/dependency-trees.sh` (ok, before the rebase), the script tests (127 passed, before the rebase) and
`openspec validate remove-inspector --strict` (valid). All prose above is written by the owner; none is left to write.
