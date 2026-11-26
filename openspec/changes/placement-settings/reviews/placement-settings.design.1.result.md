Verdict: ready for tasks

Blocking findings: none. The relaxed check holds up against the code: `ask` and `offer` compare with `<`/`>=`, `grant_waiting` returns early before `max_holders - holders.len()`, and `contest` refuses at `>= max_nodes`, so a count above its limit underflows nothing. Re-arming uses `arm`, which cancels the stale or armed id, and an old `Check` that runs later fails the `armedId` match. Adding a message inside 0.25.0 fits that version's placement contract break, which is already declared. No ADR is needed: the Drain/Gone messages were also added without one.

Notes
1. Helper shape is open (design D1). `PlacementMsg::configure()` with `with_*` methods on an enum would need methods that fail on the other variants. Name the type and the signatures, or drop the helper: the Why is the `cicada send` recipe.
2. A misspelled field next to a valid one is silently ignored: `{"maxHolder":8,"silence":"PT2M"}` changes only the silence. Consider a payload struct with `deny_unknown_fields` (check that serde allows it in an internally tagged variant), or say this in the guide.
3. D3: say that Configure applies the fields and then runs the full `check()`. That check also covers silent nodes' `heardAt`, conflict marks and contests, not only "some node". Also say whether a silence equal to the current one re-arms. `Resize` skips an equal value.
4. A shorter silence takes effect late, never early, as the pool does already. One Configure with `silence` and `maxHolders` can grant in that same turn on a node that the next turn marks silent. `end_reports` runs before the message, so a report mark past its new end lasts one more turn. Write this down, or mark the nodes silent in the same turn.
5. Docs handoff should also name: `upgrading.md` (exhaustive `PlacementMsg` matches gain a `Configure` arm); the rustdoc "fixed at its create" in `PlacementActor`/`PlacementState` and "at most maxNodes/maxHolders/maxWaiting" in `RawState`; guide line 497 ("settings are fixed"); and a caveat that the silence must stay above the members' offer interval (default 15 s), or nodes flap with an alert storm.

Questions for the owner: none.

Not checked: serde behaviour of `deny_unknown_fields` there, contract hash and fixture tests, and the next proposal beyond its text. No tests were run.
