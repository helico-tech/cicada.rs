---
status: done
created: 2026-10-07
---

# 12. Extras

## Goal

Extras: Timer, Schedule, CrashLoopGuard, FailureWatcher, DeadLetters.

## Acceptance criteria

- Feature parity with `actors-extras`; tests ported.

## Plan

Port `actors-extras` (`time/Timer.kt`, `time/Schedule.kt`, `guard/CrashLoopGuard.kt`, `supervision/*.kt`) and its
four test classes to `crates/cicada-extras`:

- `Timer`: a `Copy` value in state with `&mut self` methods (`arm` returns the new generation for the timeout
  message), the Kotlin wire form `{"generation":0,"deadline":null}`.
- `Schedule`: the Kotlin format, not cron: `Schedule.kt` defines its own (`every` seconds, `daily` hour:minute UTC),
  so it is matched exactly (wire form `{"type":"every","seconds":60}`), and `croner` is not used. A newtype over a
  private tagged enum, so an invalid schedule can neither be built nor decoded.
- `CrashLoopGuard`: an `impl Plugin` with an `on_claim` hook; `CrashLoopError` keeps the Kotlin reason text.
- `DeadLetters` (ref), `DeadLettersMsg`, `DeadLetter`, `DeadLettersState` with the Kotlin JSON names, and
  `DeadLettersActor` (`impl Actor`, capacity) in place of `deadLettersType()`.
- `FailureWatcher` over an `ActorSystem` with `poll()` and `start(interval)` (a tokio task); `dead_letter_id` is the
  same name-based UUID (v3) as Kotlin's `UUID.nameUUIDFromBytes`.

## Progress log

- 2026-10-07: Ported all five building blocks. Tests: 8 unit (timer, schedule, dead-letter id checked against Java's
  `UUID.nameUUIDFromBytes`), 5 crash-loop guard, 7 supervision (the 5 Kotlin cases plus a redrive refused by the
  type registry and a wire-form check), 5 doc tests. `cargo clippy --all-targets` clean.
- Deviations: `CrashLoopGuard::new(0)` and `DeadLettersActor::with_capacity(0)` fail at construction
  (`Result<_, String>`) instead of at node start, since a plugin has no validation hook; the texts use Rust names
  (`max_uncommitted_claims must be at least 1, was 0`). `FailureWatcher` takes an `ActorSystem`; Kotlin takes the
  `ActorOperations` interface, which its HTTP client also implements.

## Follow-ups

- [0601 An operations trait shared by ActorSystem and the HTTP client](../../issues/0601-operations-trait.md)
- [0602 `croner` is unused](../../issues/0602-croner-unused.md)
