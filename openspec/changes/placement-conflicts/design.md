# Design

Split out of `placement-member` by its design review. Two machines offering under one node id would replace each
other's entry at every offer. The pool notices a second instance under a live node's id, keeps the first, and tells
its operator once.

## Decisions

1. **Ignore, mark, alert: in an accepted turn.** An offer whose `instance` differs from the node's recorded one while
   the
   node is `live` changes no entry field (labels, places, heard time stay the first machine's). The turn succeeds: it
   sets the node's new field `conflict` to `{instance, at}` (the other instance and the store time of this offer) and,
   when that field was empty, queues one self `Alert`
   of the new kind `conflict` (in its own turn, as the other alerts; it does not repeat the instance, which the state
   holds), so the once-only mark and the alert survive. A refused (failing) turn would keep neither. A later
   conflicting offer, from the same or a third instance, only updates `conflict` to its instance and time: no new
   alert while the mark lasts. No count of ignored offers is kept.
2. **The mark ends with the conflict.** `conflict` is cleared when the node goes `silent` (its first machine stopped
   offering) or when an offer of the recorded instance comes after the pool has seen no conflicting offer for one
   silence time after `conflict.at` (the second machine stopped). A loaded state with `conflict` on a node that is not
   `live` is refused by the decode. A silent node's entry is taken over by the next offer, whatever its
   instance (a replaced machine), as today.
3. **The offer rule is modified.** "Each offer SHALL replace the node's last one" gets the exception for a live node's
   other instance. The alert kind `conflict` joins `silent` and `back`.
4. **Limits in the rustdoc.** A cloned disk with the identity file copied looks like one machine. Two machines under one
   node id also share leases and the node row in Cicada's store: the pool only reports it; fixing it is the hosts'.
5. **Inside the unreleased line; ADR 0069.** The node field `conflict` (`{instance, at}`) and the alert kind change the
   pool contract before 0.24.0 ships; contract reference and samples are regenerated. ADR 0069 extends ADR 0067 (whose
   offer rule and notice kinds it amends) without editing it. Until `placement-member` lands, `instance` is drawn at
   each start, so a restart within the silence time counts as a conflict; the member keeps it across restarts.

## Tests

On virtual time with `TestActors`: a second instance under a live node changes nothing and sends one `conflict` alert,
none for later conflicting offers; the first instance keeps renewing; after the first machine goes silent the second
takes over and the mark clears; after the second stops for one silence time the mark clears and a later conflict
alerts again; a third instance updates the mark without an alert; a decoded `conflict` on a silent node is refused;
contract and samples.

## Docs

The handoff adds the conflict alert and its limits to the guide's placement pool section, and the alert kind to the
line of `docs/architecture.md` that lists `silent` and `back`.
