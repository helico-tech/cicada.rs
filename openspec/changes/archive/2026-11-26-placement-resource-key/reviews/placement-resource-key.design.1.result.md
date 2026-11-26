Verdict: not yet

## Blocking

1. **The mark has nowhere to live** (design.md decision 3, spec "One live node holds a resource"). The mark goes on the offering node, but "a new node is not added", and a known node "keeps its last entry". A node that is ignored is never heard from, so it goes silent. `check()` then clears its mark, and the decode refuses a mark on a node that is not live ("in conflict only while it is live"). So a new node has no entry to hold the mark, and a known node loses it after one silence time. Either way the next offer sends a new `conflict` alert, so the alert repeats about once per silence time instead of once. Decide where the mark is stored: for example, add the node in a "held out" entry (status live, places not counted), or keep the mark on the holder's entry and name the other node there. Then add a scenario for a new node and one for a known node.
2. **The shape of the mark is open** (decision 3). `PlacementConflict.instance` is required, and the decode refuses a mark that names the node's own instance. identity-lock relaxes that only when `host` is set. Say what `instance` holds in a resource mark, how the decode treats it, and when a mark set by more than one rule ends ("a mark set both ways ends when both would" in ADR 0071 covers only two rules).

## Notes

- `RuntimeInfo` is `#[non_exhaustive]`, so other crates cannot set `id` through a struct literal. Name a builder (`RuntimeInfo::with_id`) for the fake and for tests.
- An empty `ID` (other daemons, or Podman's Docker API) should map to `None`. Say so.
- `GET /info` counts containers and images, so it can be slow on a busy daemon, and it now runs on every probe. Say whether that is acceptable.
- When B takes the key from a silent A, A's holders still use the daemon, so it is briefly overbooked. State this in the rustdoc.
- The decode should refuse two live nodes that hold one key.

## Questions for the owner

- None.

## Not checked

- The contract snapshots, Docker's `/info` across versions, and how this interacts with placement-settings (no clash seen in its proposal).
