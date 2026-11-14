# 0043. Deep payloads of dead letters

Date: 2026-11-14

## Status

Accepted for `deep-failure-notices`. Adds consequences to ADR 0029 without changing its depth limit or core notice.
The owner delegated design decisions to the design loop with an Opus adviser.

## Context

A Failed can be 65 levels deep and is readable by typed watchers. Dead letters keep its original payload three
containers into their state, so a payload deeper than 61 levels cannot currently be kept. User payloads can be any
JSON value: a marker inside that value would reinterpret some old letters. An extra JSON parse must not introduce
numeric rounding. Keeping a reference instead would depend on outcome retention.

## Decision

- Keep the Failed wire format and the ordinary 64-level limit. Watchers that store its payload must make room for it.
- A letter contains exactly one of raw payload (any value, including null) or payloadJson (JSON text as a string).
  Record depth <=61 raw and deeper values as text. The schema explicitly describes both alternatives and their
  exclusivity; no hidden schema fields and no in-band marker. Existing raw letters retain their meaning.
- Expose LetterPayload::{Value, Json} as DeadLetter.payload, with fallible value() decoding on demand. Bad text
  cannot block decoding the whole state. Redrive records parse/ordinary send validation errors without sending or
  replacing the kept payload. Its existing commit, capacity, metadata and once-only rules remain.
- Enable serde_json float_roundtrip through the workspace dependency. The encoding round trip preserves captured
  Value semantics, exact integers and finite f64 bits. This does not preserve original JSON bytes or undo database
  normalization, including PostgreSQL JSONB's treatment of signed zero.
- Change the dead-letter contract hash and Rust API in a breaking release. No store schema changes. Stop every old
  host before resetting the registration; startup checks do not evict already-running local hosts.
- Document a verified backup/restore maintenance recipe for independent, childless dead-letter instances with no
  incoming watches. Quiesce producers/turns, drain notices, save every instance and outgoing watch, stop hosts, reset
  registration, restore states/watches, verify and resume. Other actor topologies need a host-specific migration:
  deletion removes descendants and emits notifications, and new instances have new versions/epochs/creation times.
- Provide a tested export conversion using exact parsing. Readable legacy raw payloads may be converted to text
  before restore, even if too deep to redrive. No automatic live repair, lossy discard or business retry masquerading
  as historical notification backfill. Keep backups until verification; unreadable old data needs separate recovery.

## Consequences

Accepted deep payloads can be kept and sent again without adding nesting to state. Operators see JSON text for deep
letters; Rust consumers use value(). Old typed readers must upgrade. Systems without this actor can deploy rolling;
systems with it must plan maintenance, including all named instances and automatic bootstrap hooks.

An unquiesced upgrade has a notification gap that this change cannot fill. Redrive still applies current validation
and can reproduce the original failure; retaining a deep batch publication does not make its flow state valid.
The numeric parser improvement affects other JSON reads too, but does not promise byte identity across backends.
