# Docs handoff for heartbeat-watch

Version: 0.18.10. Refresh the dependency pins of `docs/how-to/install.md` once it is published. No change to the core,
the store schema, an existing wire format or contract hash, a guarantee, the CLI or an HTTP route; one new contract.

## What changed

A new ready-made actor in `cicada-actors`, `cicada.heartbeat-watch`, which sends a stored alarm once when a watched
thing stays silent. Public items: `HeartbeatWatch` (ref, `named`), `HeartbeatWatchActor` (unit, `Default`),
`HeartbeatWatchMsg::{Beat, Rearm, Expire}`, `HeartbeatStatus::{Waiting, Alive, Silent}` (lowercase on the wire),
`HeartbeatWatchState { silence, target, alarm, retry, status, last_beat_at, armed }` with
`HeartbeatWatchState::new(silence, &target, alarm)`, and `InvalidHeartbeat`. The rustdoc of `HeartbeatWatchActor` and
of each message states the behaviour below.

- Contract reference: `docs/reference/contracts/cicada.heartbeat-watch.json` (new). The eight existing references and
  their hashes are unchanged.
- Worked example: `examples/src/heartbeat.rs` (Example 19), tested in `examples/tests/heartbeat.rs`; routed in
  `skills/cicada/SKILL.md` under Ready-made actors.
- Tests: `crates/cicada-actors/tests/heartbeat_watch.rs` (25, virtual time), two in
  `crates/cicada-actors/tests/store_failure.rs` (real-time SQLite registry fault), state and message samples in
  `crates/cicada-actors/tests/wire_formats.rs`, decoder unit tests in `crates/cicada-actors/src/heartbeat_watch.rs`.
- `wire_formats.rs` renamed `the_contracts_of_the_eight_types_match_their_references` to
  `the_contracts_of_the_nine_types_match_their_references`; no doc names either.

## 1. The heartbeat watch in the actors guide

**Where.** `docs/guide/actors.md`: a row in the table under `Ready-made actors: choose a pattern`, and a new section
`Heartbeat watch` after `Batcher`.
**Text.** Table row: `Notice that something stopped happening` | `HeartbeatWatchActor` / `cicada.heartbeat-watch` | a link
to `../../examples/src/heartbeat.rs` labelled `heartbeat`.

Section substance:

- Create one watch per watched thing with `HeartbeatWatchState::new(silence, &target, alarm)`;
  `0 < silence <= MAX_DELAY`. Set `state.retry` for the alarm's retry at the target. The settings are fixed; delete
  and recreate to change them. A create of an existing watch changes nothing.
- A create leaves it `waiting`: no timer. Each handled `Beat` sets `lastBeatAt` to the store time of its turn,
  replaces the armed `Expire` with one due at `lastBeatAt + silence`, and sets `alive`, in one commit.
- When that timer is due with no `Beat` since, the watch sends the alarm once and becomes `silent` (`armed` cleared,
  `lastBeatAt` kept), in one commit. It arms nothing more: **one silence, one alarm**. The next `Beat` starts a new
  round; **there is no recovery message** — read `status` and `lastBeatAt`, or the existing change hints.
- To notice a thing that never sends its first sign of life, send a synthetic `Beat` with the create: create plus
  send in one turn (as `examples/src/heartbeat.rs` does in `ImportMsg::Start`), or a supervisor child's start message.
  That watch reports `alive` before any real activity.
- The alarm has the watch as sender, a new id, the stored retry policy and no correlation id; observers' ordinary send
  headers still apply (the timer gets the timer selection). Put any reply address in the alarm payload.
- Only the armed timer counts: an `Expire` from a client or another actor, a superseded one, an operator's retry or a
  dead-letter redrive (new id) ends `done` and changes nothing. An `Expire` made due early by `deliver-now` re-arms
  at the unchanged deadline and sends nothing.
- A watch whose target is itself refuses `Beat`, `Rearm` and its timer for good.
- Proposed Rust block: none needed; link the example. If a block is wanted, compile it from `examples/src/heartbeat.rs`
  (`import_watch_settings` and `ImportActor::handle`).

**Caveats.** Silence is measured from when a `Beat` was **handled** (store claim time), not when the outside event
happened or the `Beat` was sent. A `Beat` and the timer race in mailbox order `(deliverAt, seq)`: a `Beat` ahead of
the timer replaces it; a timer ahead of a `Beat` alarms first, also for a `Beat` accepted after the deadline while the
watch was paused. The due time is a lower bound: pauses, outages and slow commits make an alarm late, never early. The
alarm payload sits two levels deep in the state, so it may be at most 62 levels deep (63 is refused at create).
Changing settings, repeating the alarm, escalating, active probing and fixed clock times are not supported.

## 2. Recovery and failures

**Where.** `docs/guide/actors.md`, the new `Heartbeat watch` section; and `docs/operations.md`, wherever the other
ready-made actors' timer recovery is described (the debouncer/batcher `Flush` recovery), if anywhere.
**Text.**

- An alarm that fails commits nothing and keeps the round `alive` with its armed id: an unregistered target type or a
  message the target does not take is refused for good (`refused`); an absent target rejects the commit (`commit`);
  a failed read of the store is retried by the timer's own policy (7 attempts, 1 s to 32 s). A payload the target
  cannot decode fails later at the target; the watch is already `silent`.
- A timer that is purged, cancelled or failed for good leaves the watch `alive` with a stale `armed`: **`alive` is
  round state, not proof of a pending timer or of real health.** A purge or a pending cancel tells nobody; a failed
  timer goes to watchers and dead letters as usual.
- **Recover with `Rearm`, not with a retry or redrive.** `Rearm` arms the alive round again at its unchanged deadline
  (due at once when past) and keeps `lastBeatAt`; it also replaces a timer waiting for its retry. A retry or redrive
  of the failed timer is a copy under a new id and is ignored. `Rearm` in `waiting` or `silent` does nothing, so
  repeating it never sends a second alarm. A `Beat` also recovers, but fakes activity.
- "One alarm" means one committed enqueue per round, not exactly-once handling at the target: target retries and
  processing are independent, and an uncertain commit follows ordinary core recovery.

**Caveats.** Do not tell users the alarm proves the outside thing was quiet, only that no `Beat` was handled in time.

## 3. Architecture row

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crate table, in the list of ready-made actors after
the batcher.
**Text.** "a heartbeat watch (`cicada.heartbeat-watch`) that sends a stored alarm to a fixed target once when no
`Beat` was handled within the allowed silence, measured from the last handled `Beat`, then waits for the next `Beat`;
`Rearm` re-arms a lost timer at the unchanged deadline,"

## 4. Installation version

**Where.** `docs/how-to/install.md`, the dependency pins; and the README pins at the release.
**Text.** `v0.18.10` once the tag is published.

## Guarantee rows

No new G or S row. The new tests prove actor behaviour on top of existing guarantees and belong to no row; the
heartbeat watch relies on G2/G4 atomicity and S6 order as documented. No row names a renamed test.

## Skill route

`examples/src/heartbeat.rs` is routed now. Once the `Heartbeat watch` heading exists in `docs/guide/actors.md`, add
`docs/guide/actors.md#heartbeat-watch` to the Ready-made actors route of `skills/cicada/SKILL.md` (not added yet, so
the route test keeps passing).

## Blocking

None. Every doctest, link, guarantee row and skill route passes without prose edits.

## Status

Proposed text only; nothing is written in docs-owned files and no owner fix is integrated. Checks run on the code
branch, rebased on `baa4568` (the docs commit on `main`), PostgreSQL on port 55432 with `CICADA_PG_URL` set:
`cargo test -p cicada-examples --doc` (29 passed), `--test docs` (11 passed), `--test skill` (8 passed),
`cargo test -p cicada-examples --test heartbeat` (3 passed); `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings` (clean); `openspec validate heartbeat-watch --strict` (valid);
`cargo test --workspace` (2944 passed, 0 failed); `python3 scripts/gate.py checks --scope full` (exit 0, 3093 test
results passed, 0 failed, PostgreSQL included) and `--scope e2e` (exit 0, the Docker suite passed). Those gate
commands record no evidence; `gate.py run --e2e` was not run, as the build caller asked.
