Verdict: not yet

## Blocking

1. **Release exemption does not apply (design.md §4, proposal Impact).** The exemption covers "documentation-only updates from the dedicated docs session" (`docs/how-to/releasing.md` line 3; `docs/maintaining/maintenance.md` lines 4, 44). The same file says builders keep source and rustdoc, and that the procedure "grants no additional release permission" (lines 79-81). AGENTS.md: "Every landed implementation change is a release". The build loop always picks a version and changelog entry. A build-loop edit to `crates/cicada/src/runtime/system.rs` is producer source, not docs-session upkeep. Fix: land it as a normal patch release (0.20.5, changelog line), so docs.rs gets the fix. Extending the exemption is a new policy and needs the owner's explicit decision (Q1).

## Notes

1. "heartbeat-silence alerts" can be confused with `HeartbeatWatchActor`, which alerts on silence of explicit Beats and does not read the node table. Architecture, ADR 0054 and the actors guide call this feature "node-down alerts". Use that: "For observability, including node-down alerts. …".
2. The `openspec/specs/node-view/spec.md` Purpose still says "for display only", which contradicts its own requirement. A delta cannot change the Purpose. Add a task: fix that line when archiving.
3. ADDED is right; nothing existing is replaced. The scenario "does not claim that calling nodes publishes alerts" is tested only by reading the diff. That is fine for prose.
4. Scope, signature, contracts, guides and the no-dependency/no-link choice check out against `node_down.rs:269-277` and ADR 0054. One task is enough.

## Owner questions

Q1. Release this rustdoc fix as patch 0.20.5 (my pick), or extend the docs-only exemption to producer rustdoc edits? 0.20.5 / extend

## Not checked

Did not run cargo doc, the gate or `openspec validate`. Did not check the docs worktree for pending node-view prose.
