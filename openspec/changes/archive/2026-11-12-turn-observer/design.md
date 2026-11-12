# Design

Described on 2026-11-12 (change loop, Describe), while `control-effects` is built. The owner handed the decisions
of this change to the main session on 2026-11-11 ("Dor change 5 until 9 i fully trust you"). Every decision below is
made by the main session; each gives the options and why. Reviewed once; what changed after the review is marked
"review". Reviewed three times; what changed after the second review is marked "second review", after the third "third
review". Validated once after the build; what the main session decided on the validator's report (2026-11-12) is marked
"validation".

## Context

What the code does today (release 0.9.0):

- A node takes plugins with `ActorSystemBuilder::install`. A `Plugin` has a name and three async hooks: `on_claim`
  (after the load, before the handler; an error fails the message), `on_handler_error` and `on_outcome` (after a
  commit that succeeded). The hooks share a typed bag, `Attributes`, which the handler reads with
  `Turn::attributes`. In `on_claim` a plugin may add headers to the sends of the turn.
- `on_claim` is not called for a turn whose state cannot be decoded; `on_outcome` is. No hook is called for a turn
  that commits nothing (a lost lease, a node that stops), so a plugin finds that out when the bag is dropped.
- The plugin API has two users outside tests: `ActorTelemetry` of `cicada-otel` (a span and two metrics per turn)
  and `CrashLoopGuard` of `cicada-extras`. Chorus and Karakuri use neither the trait nor `install`.
- `CrashLoopGuard` fails a message in `on_claim` when `attempt − failures − 1` reaches its limit: the claims before
  this one that did not end in a commit. It is opt-in, and it counts every such claim, also one that a node gave
  back with a release.
- A node gives a claim back in four places: it stopped after its grace period while the handler ran; it began to
  stop while it claimed; nobody took the claim any more; it claimed a type it does not host. In the last three the
  handler never ran.
- Not every stop in good order is a release. A node that stops after a handler ended, while its commit is under
  way, or whose handler does not yield in time, leaves the lease to run out (G6: a handler that ended is never
  handed to another node by a release).
- Both stores read a message with `SELECT *`. On PostgreSQL a statement that a connection has prepared fails after
  a column was added to its table ("cached plan must not change result type") until the connection is replaced.
  Migrations 2 to 5 were all stop-first, so no release has added a column under running nodes.
- The row of an actor holds its lease and the message the lease is for. A commit and a release clear both. A lease
  that runs out stays in the row until the next claim of the actor replaces it; until then its turn can still
  commit. A cancel and a purge clear a lease that ran out only for a message that they end.
- The headers that a plugin adds count towards the limits of a send (64 headers, a name of 200 and a value of 4,000
  characters). Over a limit, the send of the handler fails.
- A claim adds 1 to `attempt`. A commit that reschedules adds 1 to `failures`. A release changes neither.
  `attempt > 1` tells a handler that an earlier run may have happened.
- A message that ends `failed` tells the watchers of its actor, in the commit that ends it (`watches`).
- The simulation of nodes under faults runs real nodes with `NodeConfig::default()`.

Chorus asks (its change `cicada-requests`, point 6): "A lost lease is not counted. A handler whose process keeps
dying is run again without a cap."

## Goals / Non-Goals

**Goals**

- One narrow trait in place of the plugin API, as ADR 0018 decides: an observer watches turns start and end and has
  no say in them.
- A message whose turns keep dying ends `failed`, in the core and by default, and a watcher is told.

**Non-Goals**

- A handler does not run inside the trace context of an observer, and reads nothing an observer made. That is as
  today: the span of `cicada-otel` was private to it.
- A metrics API in the core, or OpenTelemetry in the core.
- A claim limit per type or per message.

## Decisions

### 1. The trait: one method, and the value it returns

```rust
/// Watches the turns of a node. It has no say in them.
pub trait TurnObserver: Send + Sync + 'static {
    /// A turn starts: its message is claimed, nothing is loaded or run yet.
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn>;
}

/// One turn, as its observer holds it. Dropped without `turn_ended`: the node saw no commit of the turn succeed.
pub trait ObservedTurn: Send {
    /// The commit of the turn succeeded.
    fn turn_ended(self: Box<Self>, end: &TurnEnd<'_>);
}
```

- `TurnStart` gives the actor's address, the message as claimed (its `attempt` counts this claim) and the store time
  of the claim, and takes headers for the sends of the turn (decision 4).
- `TurnEnd` gives the outcome that was committed (`TurnOutcome`: done, failed with its error, a reschedule with its
  error and delivery, cancelled) and the messages the commit sent.
- `()` is an `ObservedTurn` that does nothing, for an observer that only adds headers. An `Arc` of an observer is an
  observer, so a test can keep a handle.
- The module `cicada::plugin` becomes `cicada::observer`.

Options for the returned value:

- **A value with one method, dropped when the node saw no commit of the turn succeed (chosen).** This is what ADR
  0018 says. The drop covers every way a turn can end without a commit, also a task that is aborted. An observer
  that cares keeps a flag and implements `Drop`.
- A second method `abandoned`. The observer needs no flag, but the core must reach that call on every path, and a
  dropped task has no path.
- A boxed closure. No second trait, but a closure cannot say what happens when it is dropped.

There is no hook for a handler error any more. The outcome carries it: a reschedule and a `failed` both hold the
error.

### 2. The methods are plain functions, not async

Today's hooks are async and the turn waits for them, so a slow hook eats the lease of the turn. An observer keeps
books: it starts a span, counts. A plain function says so in its type. It runs on a thread of the runtime and must
not block.

Not chosen: async methods, as today. Nothing in the two real users needs to wait.

### 3. When an observer is called

- `turn_started`: once for every claim of a hosted type that the node runs, before the state is decoded. So a turn
  that fails on its state or on a message that cannot be decoded is observed like any other. Today `on_claim` is
  skipped when the state cannot be decoded, and `cicada-otel` starts its span late for that case. A claim that the
  claim limit fails is the one exception: the observers are told after its commit (decision 11).
- `turn_ended`: after the commit of the turn succeeded, with what was committed. When the store rejected or refused
  the first commit and the node committed `failed` in its place, the observer is told that `failed`. A reschedule
  is an end: the next attempt is a new `turn_started`.
- Dropped without a call: the node saw no commit of the turn succeed. Its lease was lost or ran out, its node
  stopped and gave the lease back, its commit could not get through, or its actor was deleted while it ran. The
  node may not know whether a commit that it gave up on was applied; it drops the value then too, as today no
  `on_outcome` runs. So a dropped value means "no commit that the node knows of", not "nothing was committed"
  (review).
- The observers of a node are called in the order they were added, at the start and at the end.

### 4. An observer may add headers to the sends of the turn, and nothing else

`TurnStart::add_send_header(name, value)`. Every message the handler sends carries the headers. A header that a
send sets itself keeps the send's value. Of two observers that add the same header, the one added later wins. Both
rules are today's. A notification of a watch is written by the store, not sent by the turn, and carries none.

A header of an observer never fails a send (second review). Today it can: it counts towards the limits of a
message, and over a limit the handler's send returns an error, which gives an observer a say. So:

- a header that the limits of a message refuse (a name or a value that is too long or holds a NUL character) is
  left out when it is added, with a warning (third review: every header the limits refuse, not only a long one);
- a send takes its own headers first and then the headers of the observers, in the order of their names, for as
  long as the message stays within 64 headers. The ones that do not fit are left out of that message, with a
  warning.

An observer cannot refuse a turn: `turn_started` returns no error. A panic in an observer is caught and logged at
the warning level, and the turn goes on as if that observer were not there: the headers it had added in that call
are not used, the other observers are called, and the outcome is what it would have been. A panic in `turn_ended`,
or in the drop of the value, is caught the same way; the commit has happened by then.

### 5. A node takes several observers

`ActorSystemBuilder::observe(observer)`, any number of times. An observer has no name, so the start problem "Plugin
… is installed more than once" goes.

- **Several (chosen).** The builder call reads like `host`: each call adds one. It costs an order and the header
  rule of decision 4, and both exist today.
- One. Then a second call must replace the first without a word, which is a trap, or fail, which is a rule and an
  error for nothing. A host with telemetry and an observer of its own would write the code that calls both.

### 6. What leaves the public API

`Plugin`, `HookResult`, `ClaimContext`, `HandlerErrorContext`, `OutcomeContext`, `TurnInfo`, `Attributes`,
`ActorSystemBuilder::install` and `Turn::attributes`. `HandlerFailure` was public only for `on_handler_error` and
becomes private. From `cicada-extras`: `CrashLoopGuard` and `CrashLoopError`; the claim limit replaces them, and the
guard cannot be written as an observer.

Nothing replaces `Turn::attributes`. Its only users are tests.

### 7. `cicada-otel` is an observer

`ActorTelemetry` implements `TurnObserver`, with the spans, attributes and metrics it has today. Three things
change:

- The span of a turn whose state cannot be decoded starts at its claim, like every other span.
- The `exception` event is written from the outcome: a turn that ends `failed` or is rescheduled gets one with the
  error. Today a failure that is not the handler's (a rejected commit, a message that cannot be decoded) sets the
  error status of the span and writes no event; now it writes the event too.
- A turn that commits nothing ends its span with `cicada.outcome` `abandoned` in the drop of its value. The copy
  trick with the bag goes.
- A handler that failed, in a turn whose commit then does not get through, leaves a span that is `abandoned` and
  has no `exception` event (second review). Today the event is written when the handler fails, before the commit.
  The next claim of the message gets its own span.

### 8. The claim limit counts lost claims

A claim of a message is **lost** when a later claim takes its place although its turn neither committed nor gave
the lease back: its node died, its lease ran out, or its commit could not get through. A turn whose lease ran out
can still commit while no other claim came, so a claim is lost at the claim that replaces it. A node that stops in
good order but leaves a lease to run out (after a handler ended, or with a handler that does not yield) loses that
claim too (review).

Options for the count:

- **A count of lost claims on the message, written by the claim that replaces the lease (chosen, second
  review).** A claim already sees that the actor still holds a lease that nobody committed or gave back, and which
  message it was for. It adds 1 to the count of that message. There is no formula, no write in a release and
  nothing to fill in for old messages; the node and an operator read one number.
- A count `released`, written by a release, with lost claims as `attempt − 1 − failures − released` (this design
  before the second review). It needs a write in every release, a migration that fills in the past, and an operator
  who does the sum.
- Every claim without a commit, as the guard does: no new column. But a rolling deploy gives a long turn back once
  for every node it lands on, so a few deploys fail a healthy message, and three of the four releases happen
  before the handler ran. The guard could say "keep the limit well above that" because it was opt-in. A limit that
  is on by default cannot.
- A release that takes its claim back from `attempt`. No new column either, but `attempt > 1` would no longer tell
  a handler that an earlier run may have happened.

A release is a node that stopped in good order. It says nothing about the message, and it is not counted. This is
what Chorus asks for: a cap on turns whose process died.

### 9. `lost_claims` on a message

- `Message.lost_claims: u32`, 0 for a new message; `lostClaims` on the wire, `lost_claims` as a column. A plain
  `lost` says too little for a name that cannot change later (third review).
- `claim_due` adds 1 to the `lost_claims` of the message that the lease it replaces was for, in the transaction of
  the claim. That is always the message it claims, and the claim returns it with the new count: a new message
  never sorts before one that was claimed, and a cancel and a purge clear a lease that ran out when they end its
  message (third review). The store still looks the message up by the lease, and counts nothing when it is not
  pending any more.
- Nothing else changes the count: not a commit, a release, a renewal, or a lease that runs out and is not replaced.
  It never goes back to 0, also not after a retry that was committed: it counts over the life of the message.
- Migration 6 in both stores: the column `messages.lost_claims`, 0 for every message that exists. No message starts
  with lost claims from before the upgrade (review).
- The migration is not additive (review): every node of 0.10.0 is stopped first. A node of 0.10.0 that keeps
  running fails its statements on the changed table on PostgreSQL (Context), and counts no lost claim.
- The envelope of an outcome holds `lostClaims`; an envelope without it reads as 0.
- The view of a message over HTTP has `lostClaims`, next to `attempt` and `failures`; a client that reads a view
  without it reads 0. The wide table of `cicada get messages` gets the column `LOST`, and the facts of a message in
  the terminal UI the line `Lost claims`. An operator sees a message on its way to the limit before it gets there.

### 10. The limit is set on the node, and is 5 by default

`NodeConfig::max_lost_claims: Option<u32>`, `Some(5)` by default. `None` turns the limit off. `Some(0)` is a start
problem, as the other checks of `NodeConfig` are. The field is new and public: code that builds a `NodeConfig`
without `..NodeConfig::default()` adds it. The docs call the feature the claim limit, as ADR 0018 does; the field
says what it counts (second review).

- **On by default (chosen).** Without a limit a message that kills its node is claimed again for ever, and takes a
  node down each time. A message that the limit failed by mistake is in the failed outcomes, a watcher was told,
  and an operator sends it again. 5 is the default the guard had.
- Off by default, as the guard was. The safe behaviour would be the one a host has to find.

The limit cannot tell the message that kills a node from the messages that were running beside it (review). A node
that dies loses the claim of every turn it runs. A message that kills its node at once, five times, takes every
message that the node claims with it each time to the limit too, and they all end `failed` with it. That is still
better than no limit: without one the same messages are claimed and lost for ever and nothing ends. The failed
messages name the limit in their error and can be sent again; the one that fails again is the cause.

A node that is killed hard while turns run loses those claims, also when the kill is routine (a deploy that sends
SIGKILL, a Ctrl-C). The operations guide says: stop a node with its grace period, or raise the limit.

Not chosen now: running a message that has lost claims alone on its node, so that only the cause gathers more.
It would find the cause exactly, and it changes how a node claims. It can be its own change when a host needs it.

Who sets it:

- **The node only (chosen).** One number. Every node of a system should have the same; a node applies its own.
- Per type. With released claims left out, a type with long turns needs no higher limit.
- Per message, by the sender. A sender should not decide how often the receiver's node may die.

Both can be added later without a break.

### 11. How the limit ends a message

The node that claims the message checks the limit first, before it calls an observer or decodes anything. With
`lost_claims ≥ max_lost_claims` the turn does not load and does not run. It commits the message as `failed` with the
error `claim limit: <n> lost claims`, `<n>` being its `lost_claims`, through the same commit as every other outcome,
fenced by the epoch of the claim. When that commit has succeeded, the node tells its observers: `turn_started` and
then `turn_ended` with the
outcome `failed`, one after the other. The headers of such a start are not used; nothing is sent.

The observers come after the commit here (review): code of an observer that takes the process down for one message
would otherwise keep the limit from ever being reached.

- The retry policy of the message is not asked: no handler failed.
- The commit writes the `Failed` of each watcher, and the outcome is kept by the `failed` rule of the message's
  retention, as for every failed message (`watches`). `Failed.attempts` is the `attempt` of this claim.
- If that commit does not succeed, the observers are told nothing, and the next claim finds one more lost claim and
  fails the message again.
- A retry of the failed message from outside is a new message with no lost claims.
- The error starts with `claim limit:`. The docs give the text, so that a watcher can tell it from a failure of
  the handler. It is a hint and no proof: a handler can return an error with the same start (second review).
  `Failed` gets no field for it; its schema is part of the contract of every watcher.

Not chosen: failing the message inside the claim, in the store. The store would need the limit of each node, and
the notifications are written by a commit (`watches`, decision 8).

### 12. The randomised tests

- The comparison of the stores compares `lost_claims` with the rest of a message, also after a claim of an actor
  whose lease ran out.
- The simulation runs its nodes with a limit of 2 lost claims, so that generated scenarios reach it. Its checks
  learn the new end: a message may end `failed` with the error of the limit only when its log shows at least two
  claims of it that another claim replaced without a commit or a release, and no handler ran for the claim that
  failed it. Every other check holds as it is.
- Validation: the simulation also generates outages on top of its other faults. From its `nth` renewal on, every
  node fails many renewals in a row (15 to 59), so that turns lose their lease and messages lose claims. Without them
  too few generated scenarios reached the limit (1 of the fixed 200). The test of the fixed 200 asserts at least 10
  that reach the limit, and at least 10 in which a handler runs for a message with a lost claim below the limit. A
  limit drawn per scenario (1 or 2) was not chosen: it changes the spec of `randomized-testing`.

### 13. The sample service

`CRASH_LOOP_LIMIT` goes, and nothing takes its place (third review): the sample service runs with the default of
the node. No test sets the variable, and `cicada-app` names the variables of a host two changes later; its proposal
has a line on the limit.

### 14. The PostgreSQL store names its columns

Third review. Every statement of the PostgreSQL store that reads with `SELECT *` (14 places) lists its columns. A
statement that a connection has prepared then keeps working when a later migration adds a column, so such a
migration can be additive again. A test proves it on both SQL stores: a store that is open runs its operations, a
column with a default is added to each table, and it runs them again.

- **Now (chosen).** 0.11.0 is stop-first anyway, so no node with `SELECT *` is left afterwards. It is the cheap
  moment, and the cause was found here.
- A line in the upgrading text, and every later column stays stop-first.
- A change of its own. It would need a place in the order and a release for 14 lists of columns.

The SQLite store is expected to pass the test as it is: SQLite prepares a statement again when the schema changed.
If it does not, its statements list their columns too.

Validation: SQLite needed its column lists too. SQLite prepares the statement again, but sqlx keeps the columns of the
statement it prepared first: with `SELECT *`, the statement prepared again returns one more column, the worker thread
of sqlx panics (an index out of bounds) and the store answers `Unavailable(WorkerCrashed)`. So the statements of both
stores list their columns, and a source test finds no `*` in a reading statement of either store, outside `count(*)`.

### 15. A claim that arrives after its own deadline is given back

Validation. A node that receives a claim whose local deadline has passed (the time the claim was sent, plus the
lease TTL, less the safety margin) starts nothing: no observer is called, nothing is loaded and no handler runs. It
releases the claim. A release with an epoch that is no longer current changes nothing in the store, so this is safe
when another node has claimed meanwhile; when the lease is still the node's, nothing ran under it. The turn is not
observed.

- **Give it back (chosen).** A slow store, or a node that waited long for the answer of its claim, is not a fault of
  the message. The default limit of 5 must not fail messages because the store was slow.
- Leave the lease to run out, as before this decision. The node drops the turn at once because its deadline passed,
  and the claim that replaces the lease counts it as lost.

Validation, second round: a claim with less than a heartbeat of its lease left is renewed at once. A claim that
arrives shortly before its deadline still starts its handler, and the first renewal comes one heartbeat later; the
deadline passes first, the turn is dropped and the claim counts as lost. So the keeper makes its first renewal at
once when the lease it starts with has less than one heartbeat left, and otherwise waits one heartbeat, as before.
Nothing else in the keeper changes: a renewal that fails is retried as before, and a lease that the store reports lost
ends the turn as before.

- **Renew at once (chosen).** It saves the turn.
- Give back every claim with less than a heartbeat left. Simple, but it throws away claims that a renewal saves, and
  under a slow store a short handler that finishes in time today would never run.
- Leave it, and give back only a claim that arrives after its deadline. The default limit would still fail healthy
  messages under a slow store.

## Risks / Trade-offs

- A limit that is on by default can fail a healthy message: five turns of one message that each lose their lease to
  a long pause or a partition, or that each ran on a node that another message killed (decision 10). The message
  is in the failed outcomes and can be sent again. The operations guide says what counts (also a node that stops
  and leaves a lease to run out, and a node that is killed hard), what does not (a release), how to see it
  (`lostClaims`), what a message that kills its node does to the messages beside it, and how to raise the limit or
  turn it off.
- A supervisor that restarts an actor on every `Failed` would restart healthy actors after such a crash loop. The
  proposal of `cicada-actors` has a line on it.
- Every node of 0.10.0 is stopped before the first node of 0.11.0 starts, as for 0.8.0 and 0.9.0 (0.10.0 itself
  is a rolling one). The operations guide says "Version 0.11.0 is such a release", and why.
- Nodes with different limits apply their own. The operations guide says to set one value.
- An observer that blocks holds up a thread of the runtime and the turn. The docs of the trait say so.
- Telemetry writes more `exception` events than before, and none for a failed handler whose commit does not get
  through (decision 7). The changelog says so.
- A message can lose headers of an observer when it is full (decision 4). A trace then breaks at that message; the
  warning names the header.
- `Message` gets a field: code that builds one by hand must add it, as with `notification` in 0.9.0.
- The count of lost claims never goes back, so a message that lives long and is retried often can gather them over
  weeks. The limit then ends it although its last loss is long ago. The operations guide says so.

## Migration Plan

Version 0.11.0. The migration is not additive.

1. Stop every node of 0.10.0. Then start the nodes of 0.11.0; the first one migrates the database.
2. Replace `install(plugin)` by `observe(observer)`. A plugin of your own becomes a `TurnObserver`: `on_claim`
   becomes `turn_started`, `on_outcome` becomes `turn_ended` of the value it returns, and what `on_handler_error`
   saw is the error of the outcome. Code that read `Turn::attributes` has no replacement, and neither has a plugin
   that refused turns in `on_claim`: an observer has no say.
3. Remove `CrashLoopGuard`. Set `NodeConfig::max_lost_claims` if 5 is not what you want. A node that had no guard
   now has a limit.
4. Code that builds a `Message` by hand adds `lost_claims: 0`; code that builds a `NodeConfig` field by field adds
   `max_lost_claims`.
5. A watcher that matches on the error of a `Failed`: the guard's text is gone; the limit's starts with
   `claim limit:`.

## Open Questions

None.
