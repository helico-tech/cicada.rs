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
   for good and changes nothing. Anyone may send it, like `Alerts`, `Drain` and `Gone`. A helper
   `PlacementMsg::configure()` with `with_silence`, `with_max_nodes`, `with_max_waiting` and `with_max_holders`
   checks each bound when it is set. Not taken: one message per setting (four messages for one recipe, and a change of
   two limits in two turns).
2. **A lower limit keeps what is there.** Lowering a limit below what the pool has removes nothing: the known nodes,
   the holders and the waiting asks stay, and new ones are refused or queued (as today at the limit) until the count is
   below the new limit; the pool only serves under the limit. So the stored state's check changes: the nodes, holders,
   waiting asks and contests are each at most the hard cap of 1,000 instead of at most their limit. Like `Resize` of
   the permit pool. Not taken: refusing a limit below the count (the operator would first have to drain or wait).
3. **A new silence re-arms the check.** With a new silence and a check armed, the pool arms a `Check` at the new
   earliest end of a live node's silence or of a contest (cancelling the armed one, as `Rearm` does); a time that has
   passed is due at once, so a shorter silence marks the quiet nodes silent in the next turn. A conflict mark and a
   contest end one new silence time after their last offer or report. A silence whose end for some node would pass the
   pool's latest instant is refused, as in the state's check.
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

The handoff gives the guide's pool section the message, the keep-what-is-there rule and the `cicada send` recipe, and
the changelog's 0.25.0 section the message under Added.
