# Design

## One alarm per handled silence

Add `cicada.heartbeat-watch` to cicada-actors using only the public API, following ADR 0028 and the scheduler/debouncer.
No new ADR: this adds a separate actor contract, changing no existing wire shape, guarantee, core or store behavior.
The watch starts `waiting`; a first Beat starts its count. To detect a thing that never beats, create plus send Beat
in one actor turn, or create then send from a host. A supervisor may use Beat as its child's start message. This
synthetic Beat intentionally reports alive before external evidence exists. Reject Start (identical to Beat) and a
create hook/flag (actors have none). No recovery message: status/lastBeatAt and existing hints expose recovery.

## Types and validated state

`HeartbeatWatch { pub name: String }` has named(..), ActorRef TYPE `cicada.heartbeat-watch` and HeartbeatWatchMsg.
`HeartbeatWatchActor` is a Default unit actor. Tagged messages are Beat, Rearm, Expire, all without fields.
`HeartbeatStatus::{Waiting, Alive, Silent}` serializes lowercase. `HeartbeatWatchState` has public fields `silence:
Duration`, `target: ActorAddress`, `alarm: StoredMessage`, `retry: Option<RetryPolicy>`, `status`, `last_beat_at:
Option<Timestamp>`, `armed: Option<MessageId>`. Use camelCase fields and existing ISO duration serialization.
`new<R: ActorRef>(silence, &R, alarm: impl Into<R::Message>) -> Result<Self, InvalidHeartbeat>` builds waiting state;
map address/encoding errors into InvalidHeartbeat(String), and set retry through its public field as in Scheduler.
Minimal JSON supplies silence/target/alarm; retry, lastBeatAt and armed default null, status defaults waiting.

RawState/try_from validates constructor and decoded states: `0 < silence <= MAX_DELAY`; waiting has neither timestamp
nor armed id; alive has both; silent retains lastBeatAt but has no armed id. A present timestamp plus silence must be
representable and <= MAX_INSTANT. Unknown fields remain ignored. The raw schema describes shape; cross-field rules
are typed validation, not a promise that a send-only HTTP node checks them. Invalid raw state can fail later as state.
Alarm payload sits two containers into state, so new creates allow at most depth62, reject depth63 and NUL by ordinary
core value checks. The constructor does not bypass/create an extra depth budget or validate the target's availability.

## Transition table (atomic with the incoming message completion)

| Message | waiting | alive | silent |
| --- | --- | --- | --- |
| Beat | arm at claim time + silence; alive | replace timer from new claim time | arm a new round; alive |
| Rearm | no-op | replace timer at unchanged deadline | no-op |
| Valid Expire, due | impossible | send alarm; silent, armed=null | impossible |
| Valid Expire, early | impossible | replace timer at unchanged deadline | impossible |
| Other Expire | no-op | no-op | no-op |

An Expire is valid only in alive with `from == self_address` AND `id == armed`; check before any mutation. Forged,
stale, operator-retried and dead-letter-redriven copies are ignored without state writes. Normal policy retries keep
the original id/from and remain eligible. Rearm no-ops also avoid state_mut. These no-ops emit no Failed/Stopped.

Before an active action, refuse target==self permanently; otherwise an alarm could drive its own Beat/control loop.
Check this for Beat, alive Rearm and eligible Expire, including host-supplied alive state. No-op Rearm/forged Expire
stay no-ops even for self-target configuration. The decoder cannot check self without the instance address.
Compute due = time::add(lastBeatAt, silence), reject overflow/MAX_INSTANT. Beat uses turn.now() as lastBeatAt before
buffering changes. Cancel armed unless it is the current message; send_self_with Expire at(due) with the shared
7-attempt exponential 1s..32s policy, and store its new id. Beat alone updates lastBeatAt/status. No millisecond
rounding: Delivery::resolve rounds the absolute instant up to microseconds and clamps past slots to store commit time.

Eligible Expire compares claim time to the exact computed deadline. An early deliver-now therefore replaces the
timer at the deadline with no alarm. Due Expire sends alarm via send_raw, with configured target retry, a fresh id,
watch sender and no correlation id or explicit headers. Normal observer headers/telemetry still apply; do not promise
an empty envelope when observers are installed. After buffering the send, mark silent and clear armed, retaining
lastBeatAt. No further timer or recovery message. Target processing/retries occur independently of the watch.

## Failure, recovery and time meaning

Registration/message-name errors and other non-store send errors are permanent refusals; registry Store errors retain
their retryable node cause. Missing target rejects the commit. Target payload decoding may fail later at its own turn;
alarm enqueue is not successful target execution. A known failed watch turn commits no alarm/state/cancel effects.
An uncertain acknowledgement may already have committed: ordinary core fencing/recovery applies, not a second
unconditional alarm. One alarm means one committed enqueue in that round, not exactly-once target/external execution.

A purged/cancelled/exhausted timer leaves alive with a stale armed id; alive is round state, not proof of a pending
timer or actual external health. Rearm preserves lastBeatAt and recovers the incident without fabricating a Beat,
at once if overdue. It also replaces a pending retry. Terminal timer failures use ordinary watches/dead letters;
purge/pending cancel has no such notice. Operator retry/redrive cannot restore armed identity, so use Rearm after
fixing the target. Repeating Rearm cannot create a second alarm after silent, though repeated rearming can delay work.

Silence measures store claim time of handled Beats, not producer event time or send/accept time. Mailbox order is
(deliverAt, seq): a Beat before Expire replaces it; Expire before Beat alarms then the Beat starts a new round. Test
both equal-timestamp orders. A late Beat queued after the deadline can follow the alarm even when accepted while
paused. No promise that outside activity was absent. Delay/outage/pause/commit latency can make an alarm late; the
deadline guard prevents an enqueue before measured silence even after deliver-now. A new Beat envelope resets the
round; ordinary id dedup applies only within its retention window. Restart preserves state/timer, delete removes them,
repeated create leaves settings unchanged. No repeat/escalation, active probes, fixed wallclock schedule or config update.

## Proof and integration

Five tasks plus finishing: types/validation, transitions/tests, contract/failure proofs, worked example, docs handoff
and version. Use TestActors/start_paused for runtime cases, with counters/state versions proving no-op behavior;
the existing SQLite registry-fault harness keeps its documented real-time exception. Cover fractional durations,
MAX_INSTANT, depth62/63, target-self/absent/unregistered/message mismatch, observer-added headers, target retry,
lost-claim recovery, timer loss/rearm, forgery/retry/redrive and both mailbox race orders. Use existing G2/G4 atomicity
evidence for unknown-commit mechanics, not a new core test hook or a claim that every error proves rollback.

Pin the new contract and emitted states/messages, preserve all existing hashes, update wire_formats catalog counts
and keep existing named tests where docs might refer to them (otherwise hand off the affected guarantee row).
Add examples/src/heartbeat.rs: 25-hour import silence, synthetic first Beat, ongoing Beats, one alarm and recovery.
Route that compiled example in skills/cicada/SKILL.md now; add no guide-anchor link until docs prepares the heading.
Handoff the actors-guide section, architecture row, operations/recovery/time caveats, and install version to docs.
No known blocking doctest/link/route if those steps are followed. No new G/S row; report any new proof for an existing
row in the handoff. Choose the normal additive patch at build time and use the landed local-gate release workflow.
The following node-details/pinned-actors/node-down-alert proposals stay separate; this actor does not monitor node rows.
