# 0009. One baseline schema

Date: 2026-11-09

## Status

Accepted

## Context

The stores carried six migrations from the history of the Kotlin implementation, plus an unused column
(`outcomes.retried_as`) kept for rolling deploys from a version nobody runs. There are no production databases yet
(ADR 0008), so nothing needs those upgrade paths.

## Decision

Both stores have one migration, version 1, holding the final schema without `outcomes.retried_as`. The migration
mechanism stays: the next schema change is an appended version 2.

## Consequences

- Upgrade code and its tests for versions 1 to 5 are gone.
- A database written before this change (version 2 to 6) is refused as newer than supported, and must be recreated.
  A version 1 database from the old numbering would not be detected, but only the Kotlin implementation wrote those.
- From here on, the rule that a migration is never edited applies to version 1 as soon as a database that matters exists.
