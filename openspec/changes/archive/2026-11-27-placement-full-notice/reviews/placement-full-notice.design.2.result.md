Verdict: ready for tasks

Round 1's three blockers are fixed: Decision 2 keeps the contest check first (and a test covers it); Decision 1 ends the mark when an offer adds a new node (a spec scenario "The pool fills again" and a test cover it); the design settles one notice per pool. The notes are handled: the armed-check edge is in Decision 1 and the tests, placement-settings is named as extended (Decision 3, `Check`/`Rearm`/`armed` rustdoc), the handoff and ADR 0075 row are named, the rolling-upgrade note is in the changelog, and `PlacementNoticeKind` becomes `#[non_exhaustive]` (Q1 settled). I found no new blocking gap. The arming logic holds against `offer`, `check`, `arm` and `next_check`: a mark end is never earlier than a live node's end, so an already armed check is early enough.

Notes (none blocks):
1. proposal.md is stale: "To decide in the design: once per node or per pool, when the mark ends" is decided now, and "the design picks one per full period" reads oddly. Replace it with the decision.
2. The spec has no scenario for the contest-first rule or for a known node in a full pool; only the design's Tests do. Add one each, as the requirement text states the contest rule.
3. Say that `full` is `#[serde(default)]` so stored states without it still decode (no migration needed), and that `next_check` must include the mark.
4. The `full` notice's `node` is the first ignored node of the period; say so in the `PlacementNotice` rustdoc.

Questions for the owner: none.

Not checked: PlacementMember logging, contract snapshot files, E2E, the guide text.
