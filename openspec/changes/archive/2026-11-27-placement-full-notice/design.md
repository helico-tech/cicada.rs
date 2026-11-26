# Design

A pool that knows `maxNodes` nodes refuses an offer from a new node for good: the member's offer becomes a dead letter at
every interval, and nobody is told the pool is full (Factory epic 0008). The pool should take such an offer's turn,
ignore it, and say once that it is full.

## Decisions

1. **One mark per pool, not per node.** The state gains `full: Option<PlacementFull { node, at }>`: the node of the last
   offer the pool ignored because it was full, and that offer's store time. The first such offer sets the mark and
   queues one `Alert` of a new notice kind `full`, naming that node; later ones, from any node, only move `node` and
   `at`. The mark ends one silence time after `at`, removed by the check timer like a contest, or at once when an
   offer adds a new node (after `Configure` raised `maxNodes` or `Gone` freed a slot), so a pool that fills up again
   alerts again. An ignored offer arms the check when none is armed (a full pool whose nodes are all silent). Not taken: one mark per node (it needs its own bounded list, and past that bound offers would again be
   refused for good; the operator's action is the same for one node or many: raise `maxNodes` with `Configure`, or
   declare gone nodes `Gone`).
2. **The offer is accepted and changes nothing else.** Its turn commits; the node is not added and no entry changes.
   The resource check stays first: a new node offering a key another live node holds is recorded in `contested` with
   its `conflict` alert, full pool or not, as ADR 0072 says; only an offer that passes it meets the full check. A known
   node is never affected (it is not new). The contest past `maxNodes` stays refused for good.
3. **The check timer covers the mark.** `armed` and `armedId` are set exactly while a node is live, a contest is listed
   or the pool is marked full; the next check is the earliest of those ends, and the check that ends the mark clears
   `armed` when nothing else is left. A decoded `full` is checked like a contest: a node id in bounds and an `at` whose
   end is representable; the field is `#[serde(default)]`, so stored states without it decode. `next_check` includes
   the mark's end. This extends placement-settings: a new silence re-arms at the earliest end including the
   mark, and the state check on its copy covers the mark. The rustdoc of `Check`, `Rearm` and `armed` says so.
4. **The notice kind.** `PlacementNoticeKind::Full` (`"full"` on the wire), naming the first ignored node of the period
   (the `PlacementNotice` rustdoc says so), sent to the alert target with the shared
   retry policy like the others; a target that refuses it fails that alert only (during a rolling upgrade, a target on
   an older node fails it for good; the changelog says so). `PlacementNoticeKind` becomes `#[non_exhaustive]` now, so
   the next kind does not break hosts again; a match on it in a host breaks once, which the changelog names (inside
   the 0.25.0 line, whose placement contract already changes).
5. **ADR.** A short ADR 0075, extending ADR 0067 (which set the node limit but not what happens past it), records
   that a full pool ignores new nodes with one alert per full period instead of refusing their offers.

## Tests

On virtual time: a new node offering to a full pool three times commits each turn, adds nothing and sends one `full`
alert; a second new node within the silence time moves the mark and sends nothing; the mark ends one silence time after
the last ignored offer and a later offer alerts again; a known node in a full pool offers as before; after `Configure`
raises `maxNodes` the next offer adds the node and clears the mark, and a pool that fills again alerts again; a new node
offering a held key to a full pool is recorded as a contest; a full pool whose nodes are all silent arms the check on
the ignored offer and the check ends the mark and clears `armed`; a decoded state with a `full` mark and no armed check
is refused; contract reference and samples regenerated.

## Docs

The handoff gives the guide's pool section the `full` notice and what to do about it (raise `maxNodes`, or declare gone
nodes), replacing the line that says a new node's offer to a full pool is refused; the changelog's 0.25.0 section the
notice under Changed, with the `#[non_exhaustive]` kind and the rolling-upgrade note. The builder updates the
`PlacementActor` rustdoc and writes ADR 0075 with its row in `docs/adr/README.md`.
