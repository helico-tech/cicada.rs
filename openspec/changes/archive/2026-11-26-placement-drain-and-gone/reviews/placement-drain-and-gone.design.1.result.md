Verdict: not yet

## Blocking

1. **Error paths without a scenario** (specs/placement-pool/spec.md). The requirements refuse `Drain`, `Undrain` and `Gone` for an unknown node, but no scenario covers that. Two other error paths have no scenario either: a `TellGone` from another sender (it must be refused for good, like `Notify` through `refuse_foreign`), and one holder's target being gone while the others still get their notice. The design's Tests list most of these. Add them to the spec as scenarios.

## Notes

1. design.md, Decision 1: "placement-asks already counts draining nodes as unable to take places now" is wrong. On feat/placement-asks (b21dcdf8) there is no draining. `free_places` keeps only `Live` nodes, and the `noNode` check counts every node. Say instead that the builder adds the draining filter to `free_places` and leaves the `noNode` check as it is (a draining node still counts as "could ever meet").
2. Decision 2: the holder records are gone once `TellGone` runs, so `TellGone` must carry `holder`, `grant`, `node`, `to` and `messageType`. Name those fields, since they are part of the contract.
3. "Silent" should mean the node's status is `silent`, not "not heard from for the silence time". A node past its deadline stays `live` until `Check` runs. The spec scenario "offered within the silence time" mixes the two up.
4. A holder granted from the line but not yet told (`notified == false`) on a node declared gone: its pending `Notify` finds no record and does nothing, so its first message may be `gone` for a grant it never saw. The handoff should tell holder authors to accept that.
5. Say whether `Drain` of a node that is already draining, and `Undrain` of one that is not, does nothing or is refused. My pick: it does nothing.

## Questions for the owner

None.

## Not checked

The placement-holders watch-settling code (not built yet). Whether a node removed by `Gone` should send the alert target anything. Build cost against the 5-task limit: drain and gone look like about 4 tasks together.
