# Proposal

Status: done. It was found by reading the code and confirmed by a test: a debug build panicked at `time.rs:175`.

## Why

`parse_iso` in `crates/cicada/src/time.rs` (lines 159 and 175) adds each part of a duration to `total` with a plain
`+=`. `scale` keeps each part inside `i128`, but nothing checks the sum. A client can send such a duration over HTTP in
the `delay` field of a send (`crates/cicada/src/api/mod.rs:121`), or in a retry or retention policy.

## What Changes

- Add the parts with `checked_add`, so that a sum out of range is an ordinary parse error.
- Add a unit test with the input below.

Expected today:

| Build | Result of a sum out of range |
| --- | --- |
| Debug (overflow checks on) | Panic |
| Release (no `overflow-checks` in the workspace profile) | The sum wraps. Most values are then refused, but a carefully chosen three-part duration can wrap to a small positive value and be accepted |

An input with two parts that overflows: `PT47000000000000000000000000H2820000000000000000000000000M`.

### Out of scope

- A duration whose parts have mixed signs, such as `PT-1H3601S`, which is accepted as one second. It is a separate
  question.
- Other changes to the duration format.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. This fixes a bug against the existing HTTP contract. When this change is next, set `skip_specs: true` in its
`.openspec.yaml`.

## Impact

- `crates/cicada/src/time.rs` only. No public API or wire format change: an input that was already invalid is now
  always refused.
