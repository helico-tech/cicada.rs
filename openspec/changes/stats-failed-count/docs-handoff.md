# Docs handoff for stats-failed-count

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`ActorSystem::stats` returns `StoreStats::failed` as a `u64`. `GET /stats` returns the count and
`ActorClient::stats` reads it as `StatsView::failed` (`Option<u64>`). Both public structs are non-exhaustive.
`cicada stats` shows the count in the `FAILED` column or the `failed` field of JSON and YAML output.

Store conformance covers prefixes, outcome status, expiry and keep policies in
`stats_count_only_visible_failed_outcomes_under_a_prefix` and `stats_failed_count_respects_outcome_expiry_without_a_sweep`.
The HTTP test `stats_count_actors_and_pending_messages_and_name_the_oldest_due_one` also checks the new count.
`stats_documents_an_optional_failed_count` checks the OpenAPI schema.
`stats_accepts_a_response_without_the_failed_count_from_an_older_server` and
`stats_shows_the_failed_count_or_a_dash_when_the_server_omits_it` cover compatibility and CLI output.

## 1. Failed outcomes in HTTP stats

**Where.** `docs/guide/http.md`, `HTTP API`.
**Text.** `GET /stats?prefix=counter/` returns `actors`, `pending`, `oldestDue` and `failed`. The last field counts
the failed outcomes that `GET /outcomes?status=failed&prefix=counter/` would list now with no `since` filter.
An omitted or empty prefix counts across all actor addresses.
**Caveats.** A failure remains visible according to its message's keep policy, with the store's retention as the
default. A 0.26.0 server includes `failed` even when it is zero. A newer client reads a missing field as `None`.

## 2. Failed outcomes on call

**Where.** `docs/operations.md`, `On call`.
**Text.** Run `cicada stats --prefix TYPE/` for the count of currently visible failures and
`cicada get outcomes --status failed --prefix TYPE/` for individual outcomes.
**Caveats.** The count can decrease as outcomes expire or are hidden by their keep policy. When the server omits
the count, the table shows `-` and JSON or YAML shows `null`. Stats scans rows under the prefix.

## Blocking

None.

## Status

The HTTP guide and operations edits above are proposed for the docs session. The docs link and guarantee checks
passed (11 tests), and the example doctests passed (51 tests).
