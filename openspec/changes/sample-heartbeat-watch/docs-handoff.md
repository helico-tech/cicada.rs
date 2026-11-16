# Docs handoff for sample-heartbeat-watch

Version: 0.20.4. Refresh the dependency pins of `docs/how-to/install.md` once it is published. Not breaking for
library users: only the sample service changes. The libraries, actor contracts (the ready-made heartbeat-watch
contract and every earlier sample snapshot are byte for byte unchanged), wire formats, the store schema and the
guarantees are those of 0.20.3.

## What changed

- The shared sample catalog (`sample-service/src/catalog.rs`) hosts the existing `HeartbeatWatchActor` as
  `cicada.heartbeat-watch`. Both definitions use that catalog, so `cicada-sample-service`, `cicada-sample-executor`
  and the image register it under `--types all` (the default) or `--types cicada.heartbeat-watch`; `--types none`
  leaves it out. No alias, group or sample wrapper: the type is the ready-made one.
- Hosting starts nothing. Neither binary creates a heartbeat watch, sends `Beat` or arms an `Expire` at start; the
  original sample's singletons and dead-letter watch are unchanged, and the executor still has no bootstrap. A caller
  creates `cicada.heartbeat-watch/<name>` with `HeartbeatWatchState::new(silence, &target, alarm)`, makes sure the
  target exists, and sends `HeartbeatWatchMsg::Beat` to begin a round. The watch monitors handled Beats only, not the
  node table or a worker's process.
- Not added: `NodeDownMonitor` (it needs an injected source), a native node heartbeat for HTTP workers, or any timer,
  retry or dead-letter policy of the sample's own. The sample does not host every ready-made actor.
- Contract evidence: `sample-service/tests/contracts/cicada.heartbeat-watch.json` and its digest in
  `sample-service/tests/contract-texts.json`; the snapshot's parsed JSON equals
  `docs/reference/contracts/cicada.heartbeat-watch.json`.
- Tests in `sample-service/tests/sample_service.rs`:
  `both_definitions_host_the_heartbeat_watch_by_all_and_by_name_not_by_none_and_start_no_watch`,
  `a_caller_starts_a_heartbeat_round_on_the_sample` (direct store) and
  `a_caller_starts_a_heartbeat_round_through_the_executor_over_http` (send-only server and executor worker, in
  process). Before `Beat`: `waiting`, no `armed`, empty mailbox; after: `alive`, `lastBeatAt` set and one pending
  `Expire` whose id is `armed`. Timing and the alarm stay proven by the virtual-time tests in
  `crates/cicada-actors/tests/heartbeat_watch.rs`.
- No guarantee row, heading, worked example or skill route changes.

## 1. App guide, the sample service block

**Where.** `docs/guide/app.md`, `Sample service`: an addition to the block that sample-worker-mode landed in 0.20.3
(keep its executor, token and node-id paragraphs as they are).
**Text.** One short paragraph, for example after the paragraph on the executor's lack of bootstrap:

> Both binaries also host the ready-made heartbeat watch, `cicada.heartbeat-watch`, selected by `--types` like any
> other type. Hosting it creates no watch and sends no `Beat`: create `cicada.heartbeat-watch/<name>` with its
> silence, target and alarm, make sure the target exists, and send `Beat` to start a round. See
> Heartbeat watch (a link to `actors.md#heartbeat-watch`).

**Caveats.** Do not promise automatic instances, a node or worker heartbeat, or that the sample hosts every ready-made
actor: `NodeDownMonitor` is not in the catalog. The watch counts handled Beats, not wall-clock receipt or the node
table.

## Blocking

None.

## Status

- Ran: `cargo test -p cicada-sample-service` (all pass, including the three new tests and both contract tests),
  `cargo test -p cicada-actors --test wire_formats` (22 passed) and the two named virtual-time tests of
  `--test heartbeat_watch` (2 passed); the docs, doctest and skill checks and the gate are listed in the build report.
- The paragraph above is proposed text; the owner has not prepared prose yet. Task 3 stays open until it lands.
