# Design

A pool's silence, node limit, line limit and holder limit are set at its create and cannot change, so an operator who
needs another value deletes the pool and loses its nodes, holders and line (Factory epic 0008). A `Configure` message
changes them on the live pool.

```sh
cicada send cicada.placement/runners --type Configure --payload '{"maxHolders": 8, "silence": "PT2M"}'
```

## Decisions

1. **One message with optional fields.** `PlacementMsg::Configure { silence, max_nodes, max_waiting, max_holders }`,
   each `Option`; an absent field keeps its value. The bounds are the create's: silence 1 second through 1 day (an
   ISO-8601 duration on the wire, like the state), `maxNodes` and `maxHolders` 1 through 1,000, `maxWaiting` 0 through
   1,000. A value out of bounds, or a message with no field set (a misspelled field name reads as absent), is refused
   for good and changes nothing. Anyone may send it, like `Alerts`, `Drain` and `Gone`. Rust code builds the variant
   directly; no helper. A misspelled field next to a valid one is ignored, as unknown fields are in every message of
   the pool; the guide says so. Not taken: one message per setting (four messages for one recipe, and a change of two
   limits in two turns); refusing unknown fields for this variant alone (serde's attribute applies to the whole enum).
2. **A lower limit keeps what is there.** Lowering a limit below what the pool has removes nothing: the known nodes,
   the holders and the waiting asks stay, and new ones are refused or queued (as today at the limit) until the count is
   below the new limit; the pool only serves under the limit. So the stored state's check changes: the nodes, holders,
   waiting asks and contests are each at most the hard cap of 1,000 instead of at most their limit. Like `Resize` of
   the permit pool. Not taken: refusing a limit below the count (the operator would first have to drain or wait).
3. **A new silence re-arms the check.** `Configure` applies its fields to a copy of the state and runs the full state
   check on it (every node's `heardAt`, conflict mark and contest must still have a representable end); a failure is
   refused for good and changes nothing. With a silence different from the current one and a check armed, the pool
   arms a `Check` at the new earliest end of a live node's silence or of a contest (cancelling the armed one, as
   `Rearm` does); an equal silence re-arms nothing. A time that has passed is due at once, so a shorter silence marks
   the quiet nodes silent in the next turn, not in this one: a `Configure` that also raises `maxHolders` may grant a
   place on a node the next turn marks silent, and a conflict mark from `Duplicate` reports ends at the next turn. A
   conflict mark and a contest end one new silence time after their last offer or report. The guide warns that the
   silence must stay above the members' offer interval (15 s by default), or nodes flip between silent and back.
4. **A larger holder limit serves the line in the same turn** (the end of every turn already grants waiting asks while
   there is room). A larger node limit admits a new node at its next offer; offers refused before stay refused.
5. **No ADR.** The pool's rules do not change, only who may change its settings and when; the contract gains a message
   and the relaxed state check, inside the unreleased 0.25.0 line whose placement contract already changes.

## Tests

On virtual time: each field alone and together changes the state; an empty message and each out-of-bounds value are
refused and change nothing; a lower holder limit below the holders keeps them, queues new asks and grants again only
once releases bring the count below it; a lower node limit refuses a new node and keeps the known ones; a higher holder
limit grants waiting asks in the same turn; a shorter silence marks a quiet node silent at once and a longer one moves
the armed check later; a decoded state with more holders than `maxHolders` is accepted and one over 1,000 refused; the
helper's bounds; contract reference and samples regenerated.

## Docs

The handoff gives the guide's pool section (and the line that says the settings are fixed) the message, the
keep-what-is-there rule, the `cicada send` recipe, the ignored unknown fields and the offer-interval caveat; the upgrade
guide a `Configure` arm for hosts that match `PlacementMsg` exhaustively; the changelog's 0.25.0 section the message
under Added. The builder updates the rustdoc that says the settings are fixed at the create (`PlacementActor`,
`PlacementState`) or that the lists are at most their limit (`RawState`).
