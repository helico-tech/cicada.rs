# Design

## Scope

A replaying runner on the public core API; no core changes. The owner's Opus 5.5 adviser settled these choices.
`durable-flows` adds steps, `flow-sleep` adds the sleep helper, and `flow-events` adds event starts. This first change
fits five implementation tasks plus finishing checks. ADR 0031 records the replay and shared contract decisions.

## API and state

`Flow: Send + Sync + 'static` supplies `const TYPE: &'static str`, `type Input: Serialize + DeserializeOwned +
JsonSchema + Clone + Send + Sync + 'static`, `fn retry() -> RetryPolicy` (default shared crate policy), and
`fn run(&self, step: &mut Step, input: Self::Input) -> impl Future<Output = HandlerResult> + Send`.
`FlowActor::new(flow)` implements Actor with `Ref = FlowRun<F>` and `State = FlowState<F::Input>`; ordinary App hosting.
`FlowRun::<F>::named(key)` is a transparent one-string ref with a skipped PhantomData marker; manual Clone/Debug
avoid requiring F to implement them. The host chooses TYPE, unlike the fixed names of the other ready-made actors.

`Step::run<T,C,Fut>(&mut self, id: &str, call: C) -> Result<T, HandlerError>` is async;
T is Serialize + DeserializeOwned + Send, C is FnOnce(String) -> Fut + Send, and Fut is a Send future returning
Result<T, HandlerError>. The closure receives its owned idempotency key. Step owns journal/cursor/time/latch data,
not a Turn; it exposes no actor effects. No extra key helper: the closure argument suffices.
FlowMsg has only public `Continue`. No public suspension error type: HandlerResult preserves existing `?` conversions.

FlowState<I> has public input, journal: Vec<JournalEntry>, status: FlowStatus and armed: Option<MessageId>.
Journal/status/armed default to empty/running/None. `new(input)` makes that initial state;
`result<T: DeserializeOwned>(id) -> Option<Result<T, serde_json::Error>>` reads a Step result, none for absent/Sleep.
JournalEntry uses `kind`: `Step { id, result: Value }` and `Sleep { id, until: Timestamp }`, serialized step/sleep.
FlowStatus is running/done; field names are camelCase. Plain Timestamp uses its existing schema and UTC serde form.
Sleep is reserved now for the next change; it changes no schemas when its helper arrives. Not chosen: adding the
variant later (changes every flow hash), or a version/migration framework (core has no such machinery).
Example: `{"input":{"user":"u1"},"journal":[{"kind":"step","id":"account","result":"a1"}],
"status":"running","armed":"next-message"}`. Null result is valid, including a unit result.

## State validation and replay

Decode validates at most 1000 journal entries, unique ids, supported Sleep timestamps, and done implies armed None.
Ids are 1..=100 ASCII characters from `A-Za-z0-9._~-:`; run and sleep share the namespace. Unknown fields are ignored.
Running with no armed is valid for creation/recovery. Arbitrary initial journals are trusted supplied progress:
the runner checks their order and result types against flow code when replaying, not whether their effects happened.

On each execution, replay from the top with cursor zero and seen ids empty. A saved call must match the next entry's
id and kind; decode its result as T without invoking the closure, then advance. Duplicate call ids, order/kind changes,
a saved result that cannot decode, and normal return before all saved entries were reached fail permanently.
Appending a 1001st entry also fails permanently. The ordered journal was chosen over a map to detect changed replay.

## One new step per turn

Step::run is lazy until first poll. After replay/id checks, set InProgress(id) before invoking the first unsaved
closure with `<run address>/<step id>`. Keep it through awaiting, encoding and validation, replacing it without another
await by New(entry) or Failed(original HandlerError). Encoding failure, NUL or result depth over 61 latches
Permanent. A small bounded JSON walk validates the result because the core checker is private; scalar/null depth is 0.
The enclosing state/journal/entry add three levels. Input can be 63 levels deep, guarded by the core state limit.

A private Suspend error is returned to unwind via `?`. Once latched, every further Step call returns suspension
without invoking its closure. After Flow::run returns, the latch wins over its return: New commits the new entry and
next Continue together; Failed returns the original error; Permanent refuses for good. Leftover InProgress means the
step was abandoned (including a caught panic): fail permanently, naming its id, regardless of the flow's return.
No latch plus Ok completes
only after all saved entries were consumed; no latch plus Err returns that error normally.
Not chosen: dropping the flow future to hide suspension, multiple new steps per turn, or trusting only its return.
Catching Step errors cannot recover a failed step. Return a business failure as data inside a successful T to branch.
The latch is not preemption: if flow code hangs or panics after swallowing suspension, ordinary core timeout/panic
rules apply and no progress commits. Code outside closures is replayed, must be deterministic and have no external
side effects; the API cannot enforce this. A successful closure may run again if its turn never commits.
An unpolled future invokes no step and changes nothing. Put intended per-call timeouts inside the closure so their
returned error follows normal retries; timing out Step::run itself and continuing is permanent misuse.

## Public recovery and the armed continuation

Done ignores Continue. Otherwise, if its id differs from armed, cancel the old armed id if present, self-send Continue
now with F::retry() and the incoming correlation id, set armed, and commit without running flow code. This normalizes
host starts and recovery to the flow's retry policy. Only the armed message executes flow code. A suspended execution
self-sends one next Continue with F::retry(), preserves correlation, and saves its id with the journal. Never cancel
an id equal to the current message: the core refuses self-cancel. Completion clears armed and marks done.
Not chosen: executing every public Continue directly (different retry policies and duplicate continuation chains).
Only one continuation is armed; externally queued pokes can still exist. A poke intentionally resets retry attempts.
A cancelled, purged or terminally failed continuation leaves a stalled running state; another public Continue recovers.
No automatic watcher is added; hosts can watch their flow types. Pause/delete and the claim limit retain core semantics.

## Identity, evolution and integration

The closure key is stable across attempts and recovery. Step ids exclude '/', so the last slash separates them from
the canonical percent-encoded run address. Never reuse a run key for a different job, including after deletion;
recreation produces the same external keys. External deduplication is the service's responsibility. Done runs stay
until explicit deletion; there is no cleanup policy. A changed Input changes the contract; changed step code, ids,
order or result types can break replay without changing its hash. Drain old runs or use a new flow type for new code.

Keep steps.rs and add flow_onboarding.rs using the same services, with explicit create-and-Continue and App hosting.
Host it in the sample service; pin its contract and initial/progress/done samples. Actor tests pin a test flow's
contract, including Sleep, in their own fixture directory, not as a fixed ready-made type in docs/reference/contracts.
Update actors guide/doctests, README, crate docs, architecture, AGENTS map and changelog; show host contract snapshots.
Use virtual-time TestActors for replay, retries, recovery, suspension misuse, validation, limits and cancellation.
Simulate interruption before commit with public test controls and an external call counter; do not claim external
exactly-once. No step sends, invoke, parallelism, automatic subscription or state migration belongs in this change.

Review 1 found that dropping a polled step future bypassed the original latch. Opus settled a private InProgress
marker and permanent failure on abandoned steps; the spec covers cancellation and caught panics. This localized fix
changes neither the contract nor continuation protocol; a second full design review is not needed.
