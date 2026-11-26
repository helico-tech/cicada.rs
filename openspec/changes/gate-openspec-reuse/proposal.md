# Proposal

## Why

Each landing runs the full gate twice: on the builder's branch, and again on main after the archive commit, because
archiving adds `.openspec.yaml` files and review markers (`.taken`, written executable by `scripts/review.py`) that the
docs shortcut refuses. The shortcut reuses a full pass only when nothing but non-executable Markdown under the allowed
folders changed. Under load the second run costs 10 to 20 minutes per change (owner's yes, 2026-11-26).

## What Changes

- The docs allowlist gains `any_under: ["openspec/"]`: any regular file under `openspec/`, plain or executable, counts
  for the shortcut; symlinks and files outside stay refused. Nothing in the build or the tests reads `openspec/` (the
  Markdown link check skips `openspec/changes/archive` and the docs record still runs the docs checks).
- `docs/how-to/releasing.md` names the new row through the docs handoff.

## Order

First in the line. No following change.

## Impact

`scripts/gate-policy.json`, `scripts/gate.py` (`allowed_path`, `classify`), `scripts/test_gate_evidence.py`. No crate or
release.
