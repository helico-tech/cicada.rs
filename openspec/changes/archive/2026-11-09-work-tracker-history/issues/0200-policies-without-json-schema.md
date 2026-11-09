---
status: done
type: debt
created: 2026-10-07
---

# 0200. `RetryPolicy` and `Retention` have no JSON Schema

## Problem

`RetryPolicy`, `Backoff`, `Retention` and `Keep` implement serde but not `schemars::JsonSchema`. A message or state
that carries a policy (Kotlin allows this: `Request(val retry: RetryPolicy?)` in `MessageRetryTest`,
`Pass(val retention: Retention?)` in `MessageRetentionTest`) does not compile in Rust without an annotation such as
`#[schemars(with = "Option<serde_json::Value>")]`, and then its published schema says nothing about the field.

## How to reproduce

Add a field `retry: Option<RetryPolicy>` to a variant of a message family deriving `JsonSchema`: the derive fails
because `RetryPolicy: JsonSchema` is not implemented. The ported tests `crates/cicada-test/tests/message_retry.rs`
and `message_retention.rs` use the annotation as a workaround.

## Notes

- Implement `JsonSchema` for the four types by hand (their serde forms use `try_from` and custom duration formats, so
  a derive would describe the wrong shape): `maxAttempts` integer 1..100, `backoff` as a `oneOf` on `kind`, durations
  as strings (ISO-8601 or the short form) or integers (milliseconds).
- The HTTP wire model (story 08) may want the same schemas.
