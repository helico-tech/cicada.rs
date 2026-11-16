Verdict: not yet

Blocking
1. NeedsReconciliation has no defined exit (design 9, spec "A new host epoch…"). Design 9 says guidance requires
   "Stop plus confirmed terminal before deletion", but an old-epoch run can never reach confirmed terminal: no
   message closes it, and startup re-Refreshes it forever. Decide the operator exit: deletion after outside
   verification (simplest), or an explicit Abandon message that makes it terminal. Add a scenario and a test.
2. Observations after NeedsReconciliation are undefined (design 4/8). Two readings: matching-epoch Observed still
   applies, or NeedsReconciliation is absorbing. This matters in practice. Host A's shutdown pump sends terminal
   observations while successor B (same node id, new epoch) has already marked the run on its startup Refresh.
   Two live hosts with the same id (allowed by ADR 0052) also hit it. Say which transition holds, and whether
   B's mismatch marking may override a run A still owns. Then test that order.

Notes
- The order of shutdown vs ActorSystem stop is not stated. The pump needs the store open to deliver terminal
  observations.
- Stop-before-Start tombstone: durable desired Stop plus per-actor order seems enough. Say what the tombstone adds
  (it uses a slot), or drop it.
- OS test: a direct real-time `#[tokio::test]` with a timeout and an owned Child may do instead of a re-exec helper.
  The adapter's child path must be a constructor value (current_exe in the demo, the test binary in tests), never
  user input. Proposal "where supported" vs design "runs normally": name the cfg/platforms.
- Proposal Impact says "ADR if needed", while the design has ADR 0053. Align them.
- Design guide headings come from actor-design-guide (still in build). The actor-design skill route rules are not built yet.

Owner questions: none. Both blockers are delegated design decisions.

Checked: proposal/design/spec/ADR 0053 and its index row; strict OpenSpec (valid); pinned-actors design/tasks;
node-down-alert; TurnSource is public and Worker uses it; SendOptions.id dedup and NoSuchInstance;
list_actors_with_state; MessageId::random; Actor injection via Arc; examples crate layout and tokio features;
skill coverage test; maintenance ownership.
Unchecked: Worker + wrapper under paused TestActors (claimed, not tried); Tokio process docs; actor-design skill
route rules; the build.
