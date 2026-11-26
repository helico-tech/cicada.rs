Verdict: pass

## Blocking findings

None.

## Checked

- Tasks 1 and 2 are ticked, and the work for each is in the diff. The test posts `{"type", "payload"}` to
  `/actors/cicada.placement/runners/messages`. That body matches `SendRequest`, and the route is the one
  `cicada-client`'s `send_path` builds. The test checks the pool's state over `GET /actors/...` after each of
  Drain, Release, Undrain and Gone. The sequence is right: r4 waits while mac-1 drains, a release on the draining
  node grants nothing, Undrain grants r4 grant 4 on mac-1, mac-1 goes silent after 60 s while mac-2 renews, and
  Gone removes mac-1 and tells r2 and r4 once each. No failed messages.
- Scope matches the proposal: one test, no new command, no public API change. `serde_json`, `axum`, `tower` and
  `cicada-axum` are already dependencies.
- AGENTS.md rules: no version bump (the note says "none"), no protected docs edited, no generated references
  touched. No CHANGELOG entry, which fits a change that only adds a test.
- The docs handoff matches the code at head. The field names (`status` live/silent, `draining`, `places`,
  `labels`, `heardAt`, `conflict`; `holders` with `holder`/`grant`/`node`; `waiting`) match the serde names.
  "Refused for good" for Gone of a live or unknown node, and for Drain/Undrain of an unknown node, matches the
  doc comments on the messages. The `422` for a payload that does not match the schema matches `SendRequest`.
  `cicada get outcomes --status`, `get actor` and `describe actor` exist in `docs/reference/cli.md`. It lists
  nothing under Blocking, and its status says no prose is written yet, which is true.

## Notes

1. The handoff's list of node fields leaves out `instance`, the node's instance id. An operator who is
   investigating a `conflict` needs it to compare against `conflict.instance`. Worth one word in the prose.
2. After Undrain, the test checks that `waiting` is empty and reads r4's answers, but does not check `holders`
   over HTTP. That would be a small extra assertion; it is not needed.

## Not checked

I did not run the test suite or the gate (the builder reports a full gate pass of f209b09e, and the tick commit
changes only `tasks.md`). I did not run the `cicada` binary against a live node.
