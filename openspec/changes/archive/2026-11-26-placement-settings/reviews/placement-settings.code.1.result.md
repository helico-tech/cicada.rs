Verdict: pass

## Blocking findings

None.

## Notes

1. `crates/cicada-actors/src/placement.rs`, the `Configure` doc comment (and so the contract): "the nodes, holders and
   waiting asks stay" leaves out the contests, which also stay (the `PlacementActor` rustdoc and CHANGELOG name them).
2. `Ask` doc: `poolFull` "when ... the pool holds `maxHolders`" lacks the "or more" added to the other two clauses.
3. No integration test for a new contest refused under a lowered `maxNodes` while more contests are listed; the code
   (`contest`, `>=`) is right and the unit decode test covers the relaxed bound. Nice to have, not a spec scenario.
4. `CHANGELOG.md`: two re-wrapped paragraphs end lines early ("...follow Upgrading below; `NodeConfig`," and
   "...the optional `resource` to `Offer` and"); reflow.
5. The branch base is 93698992; main has two newer openspec-only commits (a3b8d615, ab42544e). Rebase before landing.

## Checked

- Tasks 1-5 ticked, each in the diff.
- Spec scenarios: lower holder limit, higher holder limit (same turn, `run_due_turns(1)`), shorter silence (next
  turn, `silent` alert, old check cancelled), no setting / `maxHolders` 0: each has a test.
- `Configure`: bounds via the create's `check()` on a copy, empty message refused, representable-end failure refused,
  no state write on equal values (version unchanged, tested), re-arm only on a different silence with a check armed.
- Relaxed state check: every use of a limit (`ask`, `offer`, `contest`, `grant_waiting`) compares with `<`/`>=`;
  the one subtraction in `grant_waiting` is guarded by the early return, so no underflow.
- Contract regenerated; CHANGELOG Added and Changed (breaking) match the code; docs handoff true to the code, Blocking
  none; no protected docs touched; versions 0.25.0; no added line over 120 columns.
- Ran `cargo test -p cicada-actors --test placement` at 08f82f5b: 62 passed.

## Not checked

Full gate, workspace tests, clippy and doctests (builder's evidence); the unit and wire-format tests were read, not run.
