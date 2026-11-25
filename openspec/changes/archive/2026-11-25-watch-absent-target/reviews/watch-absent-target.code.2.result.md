Verdict: pass

## Blocking findings

None.

## Checked (rebase resolution only, `git diff main feat/watch-absent-target`, main = base 1c5f9860, head 7423ffa7)

- Version 0.22.0 in `Cargo.toml` (workspace package), every workspace crate in `Cargo.lock` (no other lock changes),
  `.claude-plugin/plugin.json`, `cicada-version` of `skills/cicada/SKILL.md` and `skills/actor-design/SKILL.md`, and
  `info.version` of `docs/openapi.json`. No other workspace 0.21.x string remains in toml/json/skills.
- Both skills differ from main only in the version line, so main's container, permit-pool and job-graph routes are kept.
  No file of main's is lost: the diff touches only this change's files plus the version and CHANGELOG lines.
- `CHANGELOG.md`: one `## 0.22.0 — unreleased` section; one summary covering `watch_existing` and docker-context, with the
  BREAKING line; docker-context's `### Changed` entry is intact; this change's `Added`, `Changed (breaking)` and
  `Upgrading` sit in the same section. No leftover 0.21.2 heading.
- `crates/cicada-client/tests/remote_worker.rs`: `LedgerMsg` keeps main's layout (`Add` multi-line, existing variants
  unchanged); the new `Track`, `Failed` and `Stopped` variants carry doc comments; only additions in the diff.

## Notes

1. `docs/guide/containers.md:33` (docs session prose, from main) says "Context lookup arrives in 0.21.2; up to 0.21.1 ...".
   With the batch now 0.22.0, 0.21.2 will never be tagged, so this line becomes untrue at release. The docs handoff does
   not mention it; worth sending to the docs session (a one-word fix: 0.22.0) before the batch is released.

## Not checked

The rest of the change (behaviour, specs, tests, handoff content) beyond the resolution, per the request. No tests run;
I relied on the builder's reported results and gate evidence.
