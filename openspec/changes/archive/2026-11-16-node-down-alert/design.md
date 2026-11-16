# Design

The owner delegated decisions on 2026-11-16, including this batch beyond the two-ready limit and Opus 5.5 reviews.
One behavior: observe dead node rows and publish durable alerts. Five tasks plus finishing; ADR 0054 records the
new actor/message contract. Choose a ready-made monitor plus existing Topic fanout, not a new store election/outbox.

## Monitor and source

1. Add node_down.rs in cicada-actors: singleton NodeDownMonitor (`cicada.node-down/_`), NodeDownMonitorMsg,
   NodeDownMonitorState and NodeDownMonitorActor<S = ActorSystem>. One monitor address per store avoids independent
   duplicate monitors by default. Hosts create its Topic and monitor explicitly, unpinned, then send Rearm. No
   NodeConfig switch, automatic activation or extra heartbeat is introduced. The existing heartbeat-watch remains
   for explicit Beats and configurable silence of arbitrary jobs/devices; this monitor reads native node rows.
2. NodeSnapshotSource is a small public read-only dependency returning a Send future with NodeSnapshot {as_of,
   nodes: Vec<NodeInfo>} or NodeReadError. ActorSystem implements it via nodes() then now(); ignore supplied status
   and recompute against as_of. This is an observed view, not an atomic heartbeat/event log. A host can construct
   an unstarted read-only ActorSystem on the same Store before the hosting system, avoiding a self-reference. All
   hosts of the monitor must supply the same store view. No HTTP source adapter or sample-service wiring here.
3. Bound each read to five seconds with Tokio time (time-only normal dependency for cicada-actors). Source
   unavailable/timeout/invalid snapshot or more than 1000 rows: enqueue no alert, preserve dedup state, record a
   bounded lastProblem category and schedule the next poll. Do not persist raw error text. Validate unique node ids,
   representable ordered started_at <= last_seen <= as_of and bounded/NUL-free ids/details. A regressing as_of is an
   invalid read. Rejecting the whole sample is deliberate: do not clear history or infer absence from partial data.
   Polling continues and lastProblem remains visible; one bad row or >1000 rows suppresses every alert until fixed.
   The cap bounds persisted work/state, not transient allocation: nodes() reads all rows/running turns, so on a busy
   store repeated five-second timeouts may suppress alerts indefinitely. A lightweight core read is out of scope.

## Polling and alert episodes

4. State has fixed topic (typed Topic ref), alerted map keyed by NodeId with startedAt/lastSeen watermarks (<=1000),
   armed/nextPollAt, lastCheckedAt and lastProblem, all optional/default empty except topic. lastCheckedAt records
   the last successful sample as_of; success clears lastProblem. Validate paired timer fields, bounds and timestamps on construction/decode. No arbitrary payload template or list of recipients in
   monitor state: consumers Subscribe to the Topic. Removing/recreating the monitor intentionally resets its history.
5. Messages: public Rearm and private-by-convention Poll. Rearm cancels a prior armed timer unless it is this message,
   arms a self Poll at the stored nextPollAt (or now initially), and preserves alerted watermarks. Poll acts only if
   from self, its id equals armed and its deadline is due; stale/external/redriven copies are no-ops. Early delivery
   replaces the timer at the same deadline without a read. Each valid check schedules one successor ten seconds
   after turn.current_time, using the existing ready-made retry policy. All timer replacement/state is atomic.
6. A row is dead at as_of - last_seen >=120 seconds, exactly the node-view threshold. A newly observed dead row
   alerts immediately, even if never seen alive. Retain an alerted watermark for that id/startedAt/lastSeen. The
   same dead watermark never alerts again while retained. A newer lastSeen or changed startedAt permits one new
   alert when the row is dead; thus a heartbeat/re-death between polls can be distinguished without seeing it alive.
   A successful snapshot removes watermarks for absent or non-dead rows. A regressing lastSeen for the same startedAt
   invalidates the snapshot rather than rearming. No recovery notification in this version.
7. For each new dead watermark, atomically publish NodeDown to the configured Topic, store the watermark and arm
   the next poll. Concurrent hosts/lost commits use normal actor lease fencing and G2: one committed Publish per
   observed episode, not an exactly-once paging/email claim. A target type/Topic failure has normal turn/commit
   semantics: no partial watermark or alert publishes; after a failed/lost timer, repair the Topic and send Rearm.
   Do not silently drop a broken destination. Topic owns subscriber membership and isolated delivery as it does today.

## Event contract and limits

8. Export NodeDown {node, started_at, last_seen, detected_at, hostname?, cicada_version?, label?}, serde camelCase,
   RFC3339 store instants and omitted absent details. detectedAt is snapshot.as_of, not actual time of death. Use a
   private tagged NodeDown message family to construct TopicMsg::Publish/StoredMessage; consumer enums declare
   NodeDown(cicada_actors::NodeDown). The topic handles subscribers at Publish/Deliver processing time, not at detection.
   Alert messages are ordinary sends (notification=false), so normal retries/outcomes/watches apply. No new core
   watch kind or Failed/Stopped field. Omit hosted/running-turn data from alerts to keep their payloads bounded.
9. A healthy return simply updates/clears the watermark on the next successful sample; no recovery or clean-stop
   event. A clean stop removes a node row and absence is not evidence of death. Existing one-day pruning is unchanged:
   a monitor that was down may alert on retained dead rows when it resumes, but cannot report rows pruned before it
   reads them. All-cluster outage has no observer; detection resumes only with a live store and polling monitor.
   Heartbeat delay, store outages and id reuse make these liveness observations, never proof of physical failure.
10. A heartbeat after a sampled read may race the alert's commit: the reported lastSeen may already be stale when
    delivered. No second atomic node check is promised. Duplicate node ids share the existing row; startedAt is a
    row timestamp, not a globally unique host incarnation. Alerts cannot safely trigger forced pin reassignment,
    delete actors or fence outside work. Remote workers without node rows remain invisible to this monitor.

## Checks and handoff

11. Paused-time TestActors with a fake source check 119.999999/120-second boundaries, first-seen dead, same watermark,
    heartbeat then re-death between polls, new row, absence/pruning/clean stop, no recovery, duplicate/regressing
    samples, capacity/error/timeout recovery, stale/early polls and Rearm. Verify source errors rearm without losing
    history, topic removal rolls back the whole batch, and repair/Rearm delivers once. Use an unpinned monitor on
    two systems sharing a store to prove only one committed Publish; include lost-commit replay using public APIs.
12. Integration uses actual node rows/virtual clock to verify ActorSystem source and no reader heartbeat; no new
    conformance backend operation or migration. wire_formats.rs pins the monitor contract and NodeDown payload with
    a receiver sample contract, JSON/schema/state samples and generated docs/reference/contracts/cicada.node-down.json.
    Add examples/src/node_down.rs with a bounded alarm sink and Topic subscription, including missing-destination
    repair. The sink also declares Failed/Stopped and watches the monitor so terminal poll failures are visible.
    Demonstrate inspecting lastProblem separately: a caught source error is not a failed turn/watch notification.
    Keep existing actor contracts and portable core fixture cases unchanged.
13. Docs owner gets actors guide Node-down alerts plus roster, operations on-call/row-retention limits, architecture
    node-view language from display-only to observability (still never claim authority), and core node-view linkage.
    Operations must name whole-sample refusal/1000-row cap, node-read cost, failure watching versus lastProblem,
    initial alerts for retained dead rows and repeated alerts on flapping episodes without rate limiting.
    Both consumer skills route to the new guide heading/reference/example; docs-handoff.md lists those blockers and
    G2's proving test row if the new tests establish it. Integrate exact owner files before landing; no bypassed checks.
14. Builder owns actor/source trait/rustdoc, example/tests, ADR/index, generated contracts, source routes and normal
    release pins/changelog. Verify actor/wire/example/docs/doctest/skill checks, strict OpenSpec and the committed-tree
    PostgreSQL/Docker gate. No automatic subscription, paging service, worker heartbeat or node/store schema change.
