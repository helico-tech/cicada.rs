# Proposal

## Why

A multi-step job may need to wait without holding a handler open. The flow runner from `durable-flows` saves progress
but has no sleep helper. The core's store clock and delayed sends provide a durable wake-up.

## What Changes

- Add `Step::sleep(id, duration)` to the flow API. Its first execution saves a fixed wake time in the journal and
  suspends, even for zero duration. Replay returns only when store time reaches that saved time.
- Use the runner's existing Sleep journal variant, public Continue, armed-message control and retry policy. No state,
  message or schema additions, and no changes to existing contract hashes.
- An early Continue, manual recovery, retry, redrive or deliver-now cannot skip the sleep; it rearms for the same time.
- Validate duration and timestamp bounds. Document pauses/outages, cancellation and recovery through existing controls.
- Extend the flow example and guide with virtual-time tests and contract-stability checks.

## Capabilities

### New Capabilities

- `flow-sleep`: a fixed durable sleep inside a replaying flow, with no handler kept open during the wait.

### Modified Capabilities

None.

## Out of scope

Event waits, wake-time edits, cron, external effect timing guarantees, parallel steps, invocation, step sends,
new actor messages/state, and changes to the core.

## Order

After `durable-flows`, before `flow-events`.

## Impact

The flow helper/runner integration in `crates/cicada-actors`, virtual-time tests, contract-stability tests, the flow
example, actors guide/doctests and changelog. The journal representation and ADR are established by durable-flows.
