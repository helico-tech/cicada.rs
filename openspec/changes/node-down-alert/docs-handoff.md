# Docs handoff for node-down-alert

Version: 0.20.2. Refresh the dependency pins of `docs/how-to/install.md` once it is published. Not breaking: a new
ready-made actor and an example; existing actor contracts, the node table, the store schema, the heartbeat protocol,
the HTTP API and the guarantees are those of 0.20.1.

## What changed

Node-down alerts (ADR 0054): `cicada_actors::node_down`, an optional singleton monitor that reads the node table and
publishes one `NodeDown` through an existing `Topic` for each node row it sees silent for two minutes or more.

- Public items: `NodeDownMonitor` (ref, `cicada.node-down/_`), `NodeDownMonitorMsg::{Rearm, Poll}`,
  `NodeDownMonitorState { topic, alerted, armed, nextPollAt, lastCheckedAt, lastProblem }` (built with
  `NodeDownMonitorState::new(Topic)`, checked on decode, `InvalidNodeDown`), `NodeDownWatermark { startedAt, lastSeen }`,
  `NodeReadProblem::{Unavailable, Timeout, Invalid, TooManyNodes}` (wire `unavailable`, `timeout`, `invalid`,
  `tooManyNodes`), `NodeDown { node, startedAt, lastSeen, detectedAt, hostname?, cicadaVersion?, label? }`,
  `NodeSnapshot { as_of, nodes }`, `NodeReadError`, the trait `NodeSnapshotSource` (implemented by `ActorSystem`:
  `nodes()` then `now()`), `NodeDownMonitorActor<S = ActorSystem>::new(source)`, and the constants `POLL_INTERVAL`
  (10 s), `READ_TIMEOUT` (5 s), `MAX_NODES` (1000), `DEAD_AFTER` (120 s), all in `cicada_actors::node_down`.
- Setup: the host builds a reader `ActorSystem` on the same store and never starts it (no row, no heartbeat), hosts
  `NodeDownMonitorActor::new(reader)` and `TopicActor`, creates the topic, subscribes its receivers, creates the
  monitor unpinned with `NodeDownMonitorState::new(topic)` and sends `Rearm`. Nothing creates or starts it by itself:
  no `NodeConfig` switch, no sample-service wiring.
- Behaviour: a guarded self `Poll` every ten seconds (counted from the end of the check) reads the source within five
  seconds. A row is dead when `as_of - lastSeen >= 120 s`, the node view's threshold. Each dead row whose
  `(startedAt, lastSeen)` is not its watermark in `alerted` publishes one `NodeDown` (`detectedAt` = the sample time)
  as `TopicMsg::Publish`, in one commit with the watermark and the next poll. So a row dead at the first sample alerts,
  a newer `lastSeen` or another `startedAt` alerts again, and two hosts or a replayed turn publish an observed pair
  once. Absent and alive rows leave `alerted` with no recovery message. `Rearm` cancels the armed poll and arms one at
  `nextPollAt` (now on first start), keeping `alerted`; forged, superseded and redriven polls do nothing; an early
  poll re-arms at the same time without a read.
- Failed reads: an error, a read over five seconds, an invalid sample (duplicate id, id or detail over 200 characters
  or with NUL, `startedAt > lastSeen > as_of` out of order, a sample time before `lastCheckedAt`, or a `lastSeen`
  before the alerted one of the same row) or more than 1000 rows: nothing is published, `alerted` and
  `lastCheckedAt` stay, `lastProblem` gets the category, the next poll is armed. The turn succeeds, so no watch is told.
- A topic that is gone: the poll's commit is rejected (`Failed`, cause `commit`); nothing is kept and the timer is
  lost. Recreate the topic, resubscribe, send `Rearm`.
- Receivers declare `NodeDown(cicada_actors::NodeDown)` in their family (`#[serde(tag = "type")]`).
- Contract: `docs/reference/contracts/cicada.node-down.json` (generated). The topic's contract is unchanged.
- Tests: `crates/cicada-actors/src/node_down.rs` (state and sample checks), `crates/cicada-actors/tests/node_down.rs`
  (12 cases on paused time: 11 with a scripted source, and one against real node rows of the in-memory store with a
  hand-moved `VirtualClock`, where a node crashes by dropping its own runtime, which leaves its row; among them
  `a_row_silent_for_120_seconds_is_dead_and_one_a_microsecond_less_is_not`,
  `a_failed_slow_invalid_or_oversized_read_keeps_history_records_its_category_and_polls_on`,
  `a_missing_topic_rolls_the_check_back_and_repair_with_rearm_delivers_once`,
  `two_nodes_hosting_the_monitor_publish_an_observed_death_once`,
  `a_poll_whose_claim_was_abandoned_is_replayed_and_publishes_once` and
  `an_unstarted_system_reads_the_real_node_rows_without_a_row_of_its_own`), `crates/cicada-actors/tests/wire_formats.rs`
  (contract, messages, states, a receiver's `NodeDown` samples) and `examples/tests/node_down.rs`.
- Example 22, `examples/src/node_down.rs` (`cicada_examples::node_down`): `start(system)` sets up the pager
  (`ops.pager/_`, at most 100 lines), the topic `cicada.topic/node-down` with the pager subscribed, the pager's watch of
  the monitor and the monitor; `repair(system)` resubscribes and sends `Rearm`; `read_problem(system)` returns
  `lastProblem`.
- Skills (done in this change): `skills/cicada/SKILL.md`, row "Ready-made actors", and `skills/actor-design/SKILL.md`,
  row "Failure and recovery", route to `docs/guide/actors.md#node-down-alerts`,
  `docs/reference/contracts/cicada.node-down.json` and `examples/src/node_down.rs`. The first route needs item 1.
- Guarantees: no new guarantee. The one-publish tests apply G2 (atomic commit) and leases; they do not prove G2 for the
  core, so no row is required. Adding `crates/cicada-actors/tests/node_down.rs::two_nodes_hosting_the_monitor_publish_an_observed_death_once`
  to the G2 row is optional.

## 1. Actors guide: a section for the monitor, and its roster row

**Where.** `docs/guide/actors.md`: the roster table at the top, and a new section headed `Node-down alerts` (anchor
`#node-down-alerts`, routed by both skills), for example after `Heartbeat watch`.
**Text.** Roster row: `Tell people a node stopped heartbeating` | `NodeDownMonitorActor` / `cicada.node-down` |
`[node_down](../../examples/src/node_down.rs)`. Section substance:
- Setup in four steps (reader system never started on the same store; host the monitor with it and `TopicActor`;
  create the topic and subscribe receivers; create the monitor unpinned and send `Rearm`). A Rust block may be taken
  from the doc block of `examples/src/node_down.rs` (`no_run`), with `cicada_examples::node_down::start`.
- A message table: `Rearm` (start, or recover a lost timer at `nextPollAt`, keeps `alerted`) and `Poll` (own timer
  only; reads, publishes new episodes, arms the next poll; early: re-arms without a read).
- Episodes: dead at `as_of - lastSeen >= 120 s`; one alert per `(startedAt, lastSeen)` watermark; first sample alerts
  for rows already dead; newer `lastSeen` or new `startedAt` alerts again, without an alive sample in between; absent or
  alive rows are forgotten silently; no recovery message. Deleting and recreating the monitor resets its history.
- `NodeDown` fields and that `detectedAt` is the sample time, not the time of death; receivers declare
  `NodeDown(cicada_actors::NodeDown)`; Topic membership at delivery time decides who gets it.
- Two health signals: watch the monitor for `Failed`/`Stopped` (a poll that failed for good, such as a missing topic:
  repair and send `Rearm`); read `lastProblem` for caught read failures (`unavailable`, `timeout`, `invalid`,
  `tooManyNodes`), which never fail a turn.
- Links: the wire contract `../reference/contracts/cicada.node-down.json` and the implementation
  `crates/cicada-actors/src/node_down.rs`. Compare with `Heartbeat watch`: that one counts explicit Beats from your own
  jobs or devices; this one reads the native node table.
**Caveats.** An observation, not proof of a machine failure; never use it to move pins, claim or fence work. A
successful publish does not mean anyone was paged.

## 2. Operations: on-call guidance and limits

**Where.** `docs/operations.md`, `On call`: a row in the symptom table, and a few sentences in `Node table` (or a
short subsection after it).
**Text.** Symptom row: `No node-down alerts` | monitor exists, its pending `Poll`, `lastProblem`, `lastCheckedAt`, the
topic and its subscribers, failed outcomes of the monitor | `Fix the cause; recreate the topic and subscribers if gone;
send NodeDownMonitorMsg::Rearm. See [node-down alerts](guide/actors.md#node-down-alerts)`. Limits to name:
- Whole-sample refusal: one invalid row, a sample time going back, or more than 1000 rows suppresses every alert of
  that check; polling continues and `lastProblem` stays set until a good sample.
- Read cost: the source is `ActorSystem::nodes()`, which reads every node row and every running turn each ten seconds;
  on a busy store reads can time out repeatedly and suppress alerts indefinitely (`lastProblem: timeout`). The 1000-row
  cap bounds the monitor's state, not that read.
- Startup: a monitor that starts, or resumes after being down, alerts at once for every retained dead row (rows stay up
  to a day), which can be a burst.
- Flapping: each heartbeat followed by a new two-minute silence is a new episode and alerts again; there is no rate
  limit. Put one in the receiver if needed.
- Retention and races: rows removed by a clean stop or the daily pruning before a read leave no trace; a heartbeat can
  race an alert, so `lastSeen` may be stale on arrival; duplicate node ids share a row; workers over HTTP have no row;
  an all-cluster outage has no observer until a store and a polling monitor are back.
**Caveats.** Host the monitor on nodes that read the same store; one monitor per store.

## 3. Architecture: the node table is for observability

**Where.** `docs/architecture.md`: the sentence on migration 7 that calls the node details "display-only", and
`docs/operations.md`, `Node table`: "The table is diagnostic".
**Text.** Change the wording from display only to observability: the table feeds displays and sampled alerts (the
node-down monitor, ADR 0054), and still decides nothing about who runs a turn: pins select eligible owners and leases
fence turns. Node-down alerts change neither row retention nor make the table a death history.
**Caveats.** Keep the exclusion from claim authority explicit.

## 4. Core guide: link the node view to the alerts

**Where.** `docs/guide/core.md`, `Open a store and run a node` (where the node table or `nodes()` is mentioned), or
`Configuration`, row `label`.
**Text.** One sentence: to be told when a node row goes dead, host the node-down monitor; see
`actors.md#node-down-alerts`. The `label` row may say "shown in the node table and in node-down alerts".

## 5. Core guide: the examples table

**Where.** `docs/guide/core.md`, `Examples`, the table of sources.
**Text.** A row: `Alert on-call when a node stops heartbeating` → `[node_down.rs](../../examples/src/node_down.rs)`.

## Blocking

Resolved. Both skill-route tests failed on the missing anchor `docs/guide/actors.md#node-down-alerts`
(`every_route_resolves_to_a_file_and_heading_of_this_source`, `skills/cicada/SKILL.md:101`, and
`the_design_skill_routes_resolve_and_cover_the_design_guide`, `skills/actor-design/SKILL.md`). The owner prepared
`docs/guide/actors.md` in docs commit `7052813523e69d9c294c7b1ca3005f7c3600c4bb` (the roster row and the section
`## Node-down alerts`, item 1); it is taken whole, byte for byte, into the commit that adds the skill routes.

## Status

Integrated: item 1, `docs/guide/actors.md` from `7052813523e69d9c294c7b1ca3005f7c3600c4bb`. Still proposed, not yet
written: items 2 (operations), 3 (architecture and node-table wording), 4 and 5 (core guide); none of them blocks a
check. After integration: `cargo test -p cicada-examples --doc` (34 passed), `--test docs` (11), `--test skill` (18),
`--test node_down` (4), all passing; `cargo test -p cicada-actors --test node_down` 12 passed on each of 8 runs,
the real-row case 40 of 40. Before it: `cargo test --workspace --no-fail-fast` with PostgreSQL (3108 passed,
2 failed: the two skill-route tests now resolved), `cargo test -p cicada-actors` (node_down unit 7, integration 12,
wire formats 22), `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` clean,
`openspec validate node-down-alert --strict` valid.
