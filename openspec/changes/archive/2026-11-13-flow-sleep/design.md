# Design

## Scope

Add one helper to the replaying flow runner from durable-flows, following ADR 0031. The owner's Opus 5.5 adviser
settled the durable-wait policy. No core changes, new state fields, message variants or contract hashes. Three
implementation tasks plus finishing checks suffice; the original runner's reserved Sleep journal entry is used.

## Decisions

### A fixed wake time, not a handler that stays alive

`Step::sleep(&mut self, id: &str, duration: Duration) -> HandlerResult` is async and lazy until polled.
Like Step::run it returns private suspension via `?`; it never waits on a Tokio timer inside Flow::run. On a new call,
validate the id and journal limit, calculate `until = ceil(turn_claim_time + duration)`, latch a new Sleep entry and
return suspension. The runner commits that entry and a self-Continue at until, with its id as armed, in one turn.
Zero duration still saves an entry and ends the turn. This gives every new operation its own durable boundary.
Not chosen: sleeping inside the handler (holds a lease), or skipping a zero wait (different replay/commit behaviour).

The helper uses Step's captured store time. Validate `duration <= MAX_DELAY`, checked addition, microsecond ceiling,
and `until <= MAX_INSTANT`; violations latch a permanent failure with no progress. Zero is valid. MAX_DELAY must be
checked separately: a duration longer than it can still fit within MAX_INSTANT. The until field retains the existing
Timestamp representation. No runtime wall-clock time or new settings are introduced.

### Replay uses the saved deadline

At the cursor, require Sleep with the same id; other ids/kinds fail permanently. If now >= saved until, advance the
cursor and return Ok without adding an entry or timer; the flow may reach its next new operation in this turn.
If now < until, latch Waiting(until) and return suspension. The runner keeps the journal unchanged, arms a new
Continue at that same until and returns. The caller's duration is ignored on replay, including if code changed it:
the committed deadline is authoritative. All sleeps share the step-id namespace and the 1000-entry limit.
Not chosen: recomputing now + duration on replay, which could postpone a sleep forever.

### Early recovery cannot bypass the wait

Public Continue keeps the runner's normalization turn: an unarmed message cancels the previous armed continuation
and creates a new one now with F::retry(). On execution, replay sees a not-yet-due Sleep and rearms at its saved time.
Deliver-now of an armed continuation executes replay directly, but it too must rearm before until. A redrive or retry
with a new id takes the same recovery path. Never cancel the current message id. Only one continuation is armed;
there may still be other external Continue messages waiting. Existing saved steps are replayed without their closures.

If a timer is cancelled/purged or fails terminally, the run stalls until another Continue. A paused run whose deadline
passes proceeds when resumed and claimed; a late wake does not start the duration again. A next step's failure retries
that step under F::retry() and treats the preceding sleep as already elapsed. A delete removes the run and its timer;
no sleep or continuation recreates it. The deadline guarantees no early continuation, not prompt execution.

### Suspension, cancellation and compatibility

Sleep obeys any existing latch first: in particular, after an abandoned Step::run it changes nothing and cannot hide
the permanent InProgress failure. A new/replayed sleep performs its checks and latching in one poll without an await
point inside; dropping an unpolled sleep does nothing. Once latched, later Step calls cannot run a closure or add
another wait. If Flow::run swallows suspension then hangs/panics, core failure rules apply and the new Sleep is not
committed. Catching the suspension does not let a returned flow skip it: New/Waiting still decides its next turn.

JournalEntry already includes `{"kind":"sleep","id":"cool-off","until":"2026-11-14T02:00:00Z"}`.
The implementation adds the helper and scheduling of New(Sleep)/Waiting only; state/message schemas and the core's
wire formats remain byte-for-byte unchanged, descriptions included in fixture checks. No new ADR is needed.
The guide tells hosts to deploy the sleep-capable runner on every node that hosts flows using sleep before starting
such runs: an older runner cannot execute new flow code just because its schema accepts a Sleep entry.

## Integration and verification

Extend the sibling flow example with a durable wait; preserve its Input schema, type name and public state/message
contract. Show a `sleep("cool-off", Duration::from_secs(60)).await?` between external steps in the actors guide,
including early/manual recovery, cancelled timers, zero waits and changed-duration replay. Update the changelog.
Tests use virtual-time TestActors, inspecting journal, armed id and closure counters before/at/after until. Cover
manual Continue, deliver-now, new-id retry/redrive, pause, timer loss, repeated replay, zero/sub-microsecond duration,
limits/overflow, duplicate/mismatched ids, no work after an existing latch, and failure of the step after sleeping.
Run the existing flow contract/samples unchanged: a regenerated fixture difference is a failure, not an expected update.

Review 1 found no blockers. Before implementation, reconcile the helper/latch integration with the landed runner;
its APIs and fixtures are currently described dependencies. This does not change the agreed sleep behaviour.
