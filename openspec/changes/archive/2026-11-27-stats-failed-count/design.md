# Design

The inspector shows how many messages failed. `GET /stats` answers actors, pending messages and the oldest due one
under a prefix; it gains the number of failed outcomes, so a client does not page through `GET /outcomes` to count.

```sh
curl -s localhost:8080/stats?prefix=sync/   # {"actors":3,"pending":1,"oldestDue":null,"failed":2}
```

## Decisions

1. **What counts.** `failed` is the number of outcomes with status `failed` under the prefix that `GET /outcomes` lists
   now, by the listing's visibility rule without a `since`: an outcome with `visible_until` is counted while that time
   is in the future, and only then; one without it while it is within the store's retention. So a failed outcome hidden
   by its own keep policy (`Keep::Remove`, a `Keep::For` shorter than the retention, `Keep::Last` after trimming) is not
   counted, as it is not listed (design review). No prefix counts every actor.
2. **In the same read.** Each backend counts in the read transaction of the other numbers, sharing its store clock (on
   PostgreSQL each statement still sees its own committed snapshot). PostgreSQL uses the partial index
   `outcomes_failures_by_actor` (`actor_address`, `finished_at`, failed only) with the same address range as the other
   counts; the visibility test reads each such row from the table, bounded by the retention and the sweep. SQLite and
   the memory store use their own listing rules. The conformance suite checks all three against the outcome listing.
3. **The wire.** `StatsView` gains `failed: Option<u64>`: always present in a 0.26.0 server's answer, absent (`None`)
   from an older server, which a newer client must accept. `StatsView` does not refuse unknown fields when it decodes,
   so an older client reading a newer server ignores it. Additive to the HTTP API (no request, status or existing field
   changes), so no ADR; `docs/openapi.json` is regenerated. `StoreStats` gains `failed: u64`. Both structs gain a public
   field, so a struct literal outside the crate breaks: 0.26.0's BREAKING line names them, and both become
   `#[non_exhaustive]` so the next count is not a break again. `StatsView` lives in `cicada`'s `api` module;
   `cicada-axum` serves it unchanged. The terminal UI's header stays as it is (out of scope).
4. **The command line.** `cicada stats` prints the new number (table and JSON or YAML), `-` when the server did not send
   it; `docs/reference/cli.md` is regenerated if its help text changes.

## Tests

Conformance, on every backend: no outcomes counts 0; failed outcomes under two prefixes count per prefix; done and
cancelled outcomes are not counted; a failed outcome hidden by `Keep::Remove` within the store's retention is not
counted; a failed outcome past retention is not counted, as it is not listed, and one kept
by `visible_until` is. HTTP: the field in `GET /stats` and in the OpenAPI document. Client: an answer without `failed`
decodes to `None`. Command line: the new column, and `-` for `None`.

## Docs

The handoff gives the HTTP guide and operations the new number and its retention caveat, and the changelog's 0.26.0
section names it.
