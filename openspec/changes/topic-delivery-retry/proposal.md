# Proposal

## Why

`cicada.topic` sends each subscriber its message without a retry policy (`deliver` in `topic.rs` uses
`SendOptions::new()`), so a subscriber whose handler fails once ends that message failed, in dead letters. Factory's
outlets call outside systems (Jira, Teams, a results archive that can hit a full disk) and need retries (2026-11-28).

## What Changes

- A topic's deliveries carry a retry policy, so a subscriber's failed turn is tried again before the message fails.

To decide in the design: per subscriber (given in `Subscribe`) or per topic, and the default (none, as today, or the
crate's shared policy); the contract change of `cicada.topic` (a new optional field) and its reference in
`docs/reference/contracts`.

## Out of scope

- Retrying the topic's own `Deliver` self-sends: they already have the shared policy.
- Ordering between subscribers or between deliveries.

## Order

After `work-id-header`. Before `container-pull-policy`.

## Impact

Crates: cicada-actors.

`crates/cicada-actors/src/topic.rs` and its tests, `tests/wire_formats.rs`, `docs/reference/contracts/cicada.topic*`
(regenerated). Docs handoff: the actors guide's topic section.
