Verdict: not yet

## Blocking

1. **When is a waiting ask's target watched?** Design decision 1 and the spec requirement watch a target "when it records the holder" (at the grant). But decisions 2 and 3 have waiting asks removed by `Stopped`, and count watches as up to `maxHolders + maxWaiting`. The spec scenario "A run deleted before its grant" says the ask "gets no place" and "the place stays free". That only holds if the target is watched when the ask is queued. If it is watched only at the grant, the line grants the place, `watch_existing` writes a `Stopped`, and the self `Notify` fails too. The place is held for a moment and freed twice (the second free does nothing). Fix: call `watch_existing(to)` in the commit that queues the ask as well, and say so in decision 1 and the requirement. The line grant then keeps the existing watch.

## Notes

1. Do the watch changes once, at the end of the turn: compare the targets before and after, `watch_existing` the new ones, `unwatch` the ones that went. Watch calls apply in order and the last one stands. So a `Release` and a line grant to the same target in one turn can otherwise leave the target unwatched.
2. If an ask's `to` is the pool itself, decision 3 unwatches the pool's own address. That removes the self-watch that frees undelivered grants (placement-asks). Never unwatch the pool's own address, or refuse such an ask.
3. A watch on an address also sends a `Failed` for every failed message of the holder (architecture, Notification). That costs the pool one turn per failure. Decision 4 should say a `Failed` about another actor changes nothing (`undelivered` already ignores it), and a test should check it.
4. The spec has two scenarios. The design's tests also cover two holders with one target, unwatch after the last release, `Stopped` of the pool or of an actor that holds nothing, and a line grant to a target that is already gone (`Stopped` plus the failed `Notify`: freed once). Add these as scenarios so "exactly once" can be tested.
5. Decision 1's case against a plain `watch` is weaker than it says: the failed `Notify` already frees a line grant to a gone target. `watch_existing` is still the right choice; just say why correctly.

## Questions for the owner

None.

## Not checked

Locking in the PostgreSQL store, mailbox order after address reuse (a run deleted and re-created with the same address), the later proposals beyond their Order sections, and any doc blocks.
