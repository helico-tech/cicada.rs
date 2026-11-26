# Design

The third change of the placement-pool line. A holder that is deleted without a `Release` would keep its place for
good (Factory issue 0014). `watch-absent-target` (0.22.0) gave the core `Turn::watch_existing` for exactly this.

## Decisions

1. **The pool watches each holder's target with `watch_existing`.** The holder is the actor named by an ask's `to`
   (where its answers go). The pool watches it from the commit that first records the ask, whether it is granted at
   once or queued (`turn.watch_existing(to)`); a later grant from the line keeps that watch. So a target deleted while
   its ask waits is seen, and its ask never gets a place. When that actor is deleted, before or after the commit, the
   pool gets one `Stopped`
   for it (`watch_existing` sends `Stopped` at once when it is already gone), and frees every place held by holders
   whose `to` is that actor; freed places go to the line as after a `Release`. Not taken: a plain `watch` (an actor
   deleted before the commit would never send `Stopped`, which is the bug); polling the targets (slow, and a turn per
   holder). A plain watch would also miss a target deleted before the commit; the failed self `Notify` frees a line
   grant to a gone target, but not a place granted at once, so `watch_existing` is the one that closes every case.
2. **Waiting asks of a gone target leave the line.** The same `Stopped` removes waiting asks whose `to` is that actor,
   so a deleted run does not get a place later.
3. **Watches follow holders, settled once per turn.** At the end of each turn the pool compares the targets of its
   holders and waiting asks before and after, and watches the new ones and unwatches the ones that hold and wait for
   nothing; so a `Release` and a line grant to one target in one turn leave it watched (watch changes apply in order
   and the last stands). Its watches stay as many as its distinct targets (at most `maxHolders` + `maxWaiting`). An
   ask whose `to` is the pool itself is refused for good, so the pool never unwatches its own address, which frees
   undelivered grants.
4. **Telling notifications apart.** The pool already watches itself (for undelivered grants). A `Stopped` for its own
   address is ignored; one for another actor frees that actor's places. A `Stopped` whose actor holds nothing (a
   watch already removed, a repeat) changes nothing. Watching a holder also brings a `Failed` for every failed message
   of that holder: a `Failed` about another actor changes nothing (it costs a turn; the handoff says so).
5. **Additive inside the line.** No new message or state field: `Stopped` is in the family since `placement-asks`, and
   holder records keep `to`. It ships with the line in 0.24.0.

## Tests

On virtual time with `TestActors`: a holder deleted after its grant frees the place and the line gets it; a holder
deleted before the turn that grants from the line gets no place and frees it at once (`watch_existing` of a gone
target); a waiting ask of a deleted target leaves the line and was watched since it was queued; a release and a line
grant
to one target in one turn leave it watched; an ask to the pool itself is refused; a `Failed` about a holder changes
nothing; two holders with one target are both freed; a release of
the last holder unwatches its target (the store lists no watch); a `Stopped` for the pool itself or for an actor that
holds nothing changes nothing.

## Docs

The handoff adds to the guide's placement pool section: a deleted holder frees its place, so a holder actor need not
release before it stops.
