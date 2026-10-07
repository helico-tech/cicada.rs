---
status: todo
type: idea
created: 2026-10-07
---

# 0701. `Timestamp` and `MessageId` in state need manual schema and serde attributes

## Problem

Kotlin states hold `Instant` and `MessageId` fields directly. In Rust, `Timestamp` (chrono `DateTime<Utc>`) has no
`JsonSchema` (schemars' `chrono04` feature is off) and is not serialized in the Kotlin form by default, and `MessageId`
has no `JsonSchema`. Every such field needs attributes:

```rust
#[serde(with = "cicada::time::serde_instant")]
#[schemars(with = "String")]
pub registered_at: Timestamp,

#[serde(default)]
#[schemars(with = "Option<String>")]
pub armed_tick: Option<MessageId>,
```

Seen in `examples/src/registry.rs` (`DeviceState`), `examples/src/cron.rs` (`CronMsg::Tick`, `CronState`) and
`cicada-extras` (`Timer`, `DeadLetter`).

## How to reproduce

Derive `JsonSchema` on a state with a `MessageId` field: it does not compile.

## Notes

- `MessageId`: derive or implement `JsonSchema` (a string) in `crates/cicada`. Small and safe.
- `Timestamp`: a newtype with serde and schema in the Kotlin form would remove both attributes, but changes a public
  type used everywhere; decide in an ADR before doing it. Keeping the attributes is acceptable.
