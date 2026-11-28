# 0031. Durable flows replay an ordered journal

Date: 2026-11-13

## Status

Accepted for `durable-flows`. The owner delegated remaining design decisions to an Opus 5.5 adviser.
Partly superseded by [ADR 0080](0080-flow-runs-end-with-an-output-or-a-failure.md): how a run ends.

## Context

A multi-step job can already be an actor whose turns save results and send the next step. The worked steps example
shows this correctly, but each host repeats the state machine. A flow can write the order as async code and replay
saved results, while keeping the existing atomic commit and at-least-once execution guarantees.

The original proposal combined a runner, durable sleep and event starts. These are three small releases. Since
contract hashes are strict, the runner's initial journal must include the immediately planned sleep representation.

## Decision

- `FlowActor<F>` is an ordinary layer-3 actor on the public core API. Its type name is chosen by F, an exception to
  ADR 0018's fixed names for ready-made actors. A typed FlowRun ref names one run. App needs no new hosting method.
- A Flow supplies typed input, async run code and a retry policy. A Step call returns a saved JSON result in journal
  order, or executes one new closure. The new result and the next Continue commit together. Results may be null.
- The state wire shape is input, journal, status and armed. Journal entries are tagged step (`id`, `result`) or sleep
  (`id`, `until`) from the first release; status is running/done. Durable sleep's helper comes in the next release
  without changing this shape. At most 1000 entries are kept. Step ids are unique in a run, 1..100 ASCII characters
  from `A-Za-z0-9._~-:`. State decoding checks these invariants; a done run has no armed continuation.
- Replay checks the ordered id, entry kind and result decoding. A duplicate call id or a normal return before all
  saved entries are reached fails permanently. Host-supplied initial progress is trusted, not a proof of effects.
- Flow code returns the core's HandlerResult. Step returns a private suspension error carried by `?`, but a private
  latch records the first new entry or failure. Further Step calls run no closure. Once the flow returns, the latch
  decides the turn even if the code caught that error. A business failure used for branching is returned as data.
  The latch does not preempt code: a swallowed suspension followed by a hang/panic uses ordinary core failure rules.
- Before invoking a new closure, Step records InProgress through awaiting, encoding and validation. Dropping that
  future, or catching a panic in it, leaves the marker: later Step calls invoke nothing and the returned flow fails
  permanently without progress. An unpolled future does nothing. Intended per-call timeouts belong inside closures.
- Public Continue starts and recovers a run. An unarmed incoming message first cancels the old armed message and
  arms a self-Continue with F's policy, without executing flow code. Only the armed continuation replays. Each
  suspension saves the next armed id; never cancel the current message. A public poke intentionally resets retries.
- Code outside step closures is deterministic and free of external effects. Closures may execute again before their
  result commits; core guarantees remain unchanged. Each receives `<run address>/<step id>` as an idempotency key.
  Canonical addresses and ids without slash make the key unambiguous. Hosts must not reuse a run key for another job,
  even after deletion; the same key produces the same external idempotency keys. External dedup is the service's job.
- Done runs remain readable and ignore Continue until explicitly deleted. A lost or failed continuation leaves a
  stalled running state; another Continue recovers it. Hosts use ordinary watches, reads, pause, resume and delete.
- Hosts pin each flow's contract. Input schema changes follow the existing contract-change procedure. Step order,
  ids, result types and code can change replay without changing a hash: finish old runs or use a new type name.

## Consequences

- No change to G1–G6, S1–S10, HTTP or the store schema. There is no promise of exactly-once external execution.
- One new step per turn, plus a start/recovery turn. Replay rereads the whole bounded journal; throughput is not a goal.
- Result payloads can be 61 container levels deep (state, journal and entry consume three); input can be 63. Invalid
  new results fail permanently before they are saved. There is no bound on bytes beyond existing core/HTTP limits.
- The Sleep variant is intentional advance definition for the next agreed change, not a generic extension framework.
- `FlowState` exposes data, not protection against a host inventing progress. Deterministic replay is a host obligation.
- The explicit state-machine example stays beside a flow example. The architecture and actors guide describe this
  layer-3 exception when the runner is implemented. No automatic migration, cleanup, step sends or parallel steps.
