# Design

## Context

See `proposal.md` for why, and `specs/randomized-testing/spec.md` for what must hold.

Facts that shape the approach:

- Nodes coordinate only through the `Store` trait. There is no network between them, so a wrapper around that trait can
  produce every failure a node can see.
- `TestActors` and the runtime already run on paused tokio time with `InMemoryStore` and `TokioClock`. Two nodes on one
  store on paused time already exist in `crates/cicada-test/tests/lease_robustness.rs`.
- `SqliteStore` takes a `Clock`, so it can run on a virtual clock, as its conformance fixture does.
- `Faulty` and `Faults` in `crates/cicada-test/tests/common/mod.rs` intercept five operations and are private to those
  tests.

## Goals / Non-Goals

**Goals:**

- One crate, one tool, and the existing `cargo test` as the way to run it.
- A failing case that is a short, readable list of steps.

**Non-Goals:**

- Measuring coverage, or a target for it.
- Replacing the conformance suite or the Docker fault tests. They stay the examples; this searches around them.

## Decisions

### One crate, `crates/cicada-fuzz`

Shared code in `src/` (generators, fault layer, checks). One test file per kind in `tests/`: `parsers.rs`, `backends.rs`,
`simulation.rs`. No library crate depends on it.

### proptest for all three

It runs on stable inside `cargo test`, shrinks a failing case, and saves failing cases.

- `cargo-fuzz`: stronger on parsers, but needs nightly, sits outside CI and fits async code poorly.
- `madsim`, `turmoil`: they replace tokio or fake a network, and nodes have no network between them.
- A plain seeded loop: simple, but a failure stays a long run that nobody can read.

Parser generators are shaped as well as random (for example many-digit numbers followed by a unit), because purely random
text rarely reaches the arithmetic.

### A scenario is a value

proptest generates a `Scenario`, not only a seed, so that it can shrink it:

```
Scenario
  nodes:   1..=3
  actors:  a few addresses of one generic actor type
  steps:   client calls and time advances
  faults:  (node, operation, nth call of that operation, fault kind)
```

There is one actor type. A message's payload is its script: a list of actions such as send, create, stop, sleep, fail or
panic. New behaviour then needs a new action, not a new actor. Every script action also appends the message id to the
state, which is what the G4 check reads.

Faults are keyed by call count and not by time, as call counts stay exact on replay.

### The fault layer covers every store method

`FaultStore` implements `Store` for one node and sends every method through one gate, which applies the node's fault rules
and writes to the event log. `Faulty` is not reused: it covers five operations and cannot be imported from another crate,
and widening it would give up its simple per-test shape.

| Fault | Behaviour |
| --- | --- |
| Before | Returns `Unavailable`; nothing applied |
| After | Calls the inner store, then returns `Unavailable` (lost acknowledgement) |
| Delay | Sleeps on virtual time, then calls the inner store |
| Dead | This call and all later ones return `Unavailable` |

Dead stands in for a crash. It is harsher than a real crash: the node's handlers keep running until its watchdog stops
them, as in a network split.

### Checks read an event log, not the store alone

The fault layer records what the inner store really answered, including answers the node never received. Handlers record
start and end. Then:

| Check | Evidence |
| --- | --- |
| G1 | Handler start and end per instance; successful commits per epoch |
| G3 | Every message the inner store accepted has one final store event (commit, cancel, purge or delete), and exactly one outcome after the drain |
| G4 | The ids in the state have no duplicates and are all `done` |
| G5 | At each claim: delivery time is due, and the claimed message is the head of `pending` |
| G6 | Handler runs per message, against its retry policy and the turns lost before a commit |
| S8 | After the drain bound, no due message is left pending |

"Accepted" comes from the inner store, because a client can get an error for a send that applied.

The drain step lifts all faults except Dead, resumes paused instances and advances virtual time past the largest possible
lease, delay and backoff in the scenario. Generators keep those small. Runs use a long retention and no hidden-outcome
policy, so that no outcome disappears before the check. Every scenario keeps at least one node that is never Dead.

### Backend comparison runs one operation at a time

One list of operations with a small pool of addresses and ids, so that duplicates and conflicts are common. Both backends
are driven in step, with one virtual clock moved by hand, on normal (not paused) tokio time, because sqlx does real I/O.

Comparison rules:

- `seq` and `epoch` are compared by rank within each backend, not by value.
- Errors are compared by kind (`Unavailable`, `Invalid`, `Corrupt`), not by text.
- Each backend pages with its own cursors, and the full pages are compared.
- Which instances one claim takes among those that are due is free, as S6 promises an order per instance only. SQLite
  breaks ties in `next_due_at` by its index and `InMemoryStore` by address. So a claim asks for every due instance, and
  the claims are compared sorted by address. Found by the first run.
- The reasons of `Rejected` and `Conflict` are not compared. They are text for people, like the text of an error.
- Any other allowed difference is a named rule with a written reason. Everything else is a finding.

### Replay needs `biased;`

The three `select!` calls at `executor.rs:350`, `system.rs:433` and `system.rs:622` choose a branch at random when two are
ready. `biased;` makes them deterministic, as `executor.rs:247` already is. In all three a ready branch ends the wait, so
none can starve the other.

The alternative, `Builder::rng_seed`, needs `--cfg tokio_unstable` and would make the fuzz build differ from every other
build.

Scripts and client calls pass explicit message ids and node ids, so random UUIDs do not enter a run.

### A found bug goes green before this lands

A saved failing case fails `cargo test` until it is fixed, and CI must stay green. So:

1. A failure found while building this becomes its own change with a `proposal.md`, holding the shrunk case.
2. That change is fixed and merged first, together with its saved case.
3. No check is weakened and no case is excluded to get green.

`fix-duration-overflow` is the first of these and goes before the parser step finishes.

## Risks / Trade-offs

- [The simulation trusts `InMemoryStore`] → The backend comparison checks it against SQLite. It is built first for this
  reason.
- [Random search can miss rare orderings] → Long runs on demand with `PROPTEST_CASES`. Faults are generated around the
  operations that matter (commit, renew, claim).
- [SQLite cases are slow, so CI takes longer] → A low default case count for the comparison test. The number is set from
  measured time.
- [A false G1 alarm when one handler stops and the next starts at the same virtual instant] → Treat the first such report
  as a question: the architecture says turns never overlap, so the check stays strict unless the investigation shows the
  tie is harmless. Then it becomes a written rule.
- [Scenarios that stop being replayable, from a new source of randomness in the runtime] → A test runs one scenario twice
  and compares the event logs.
- [A generator change invalidates saved cases] → A saved case that still matters is also added as a plain test by its fix
  change.

## Migration Plan

Test-only. `biased;` is the only production change, and rollback is reverting it.
