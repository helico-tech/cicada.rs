Verdict: pass

## Blocking findings

None.

## Notes

1. Three doc-comment lines in `crates/cicada-actors/src/placement.rs` are longer than the 120 columns of
   `rustfmt.toml` (rustfmt does not wrap comments, so the gate passes): the `PlacementMsg` doc ("...a holder id is 1
   through 200"), the `PlacementActor` drain paragraph ("...in a `TellGone` that the pool sends") and the doc of
   `place` ("...the one with the most free places,"). Rewrap them.
2. `Drain`/`Undrain` check the node id inside `drain()`, `Gone` checks it in the handler before `declare_gone()`. Both
   are right; one place would read more evenly.
3. `TellGone` checks its fields before `refuse_foreign`, as `Alert` and `Notify` do: a foreign one with bad fields is
   still refused for good, so this is only order.

## Checked

- Tasks 1-4 ticked; each one's work is in the diff (drain flag and `free_places` filter; never-meet check still counts
  draining nodes; offer keeps `draining`; `declare_gone` removes the node and holders, one self `TellGone` with retry
  per holder; changelog; regenerated contract; handoff).
- Every spec scenario has a test (drain with work, gone of silent/live/unknown, foreign `TellGone`, one deleted target).
- Invariants: `armed` stays right (only a silent node is removed); `check_books` holds (holders of the removed node
  go with it); `follow` unwatches told holders, so a deleted target gives no `Stopped`; a pending `Notify` finds no
  holder and sends nothing, as documented; a late `Release` cannot match a new grant.
- Versions 0.24.0 in `Cargo.toml`, both skills and `plugin.json`. No protected docs edited; only the generated
  contract in `docs/`. The handoff is true to the code, and its targets (guide answer-kind table, architecture row)
  exist.

## Not checked

I did not run the tests or the gate; I relied on the builder's report and the gate evidence for 0e6c4405. I did not
check the full schema for `PlacementAnswer` outside the wire test.
