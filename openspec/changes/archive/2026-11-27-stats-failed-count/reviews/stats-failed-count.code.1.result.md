Verdict: pass

## Blocking

None.

## Checked

- Tasks 1-5 ticked, each in the diff. Every spec scenario has a test: prefixes and status (conformance
  `stats_count_only_visible_failed_outcomes_under_a_prefix`, HTTP stats test), past retention, `Keep::For` shorter and
  longer than retention (`..._respects_outcome_expiry_without_a_sweep`), `Keep::Remove`, `Keep::Last` after a trim
  (the count stays 3, so the trim is seen), older server (client and CLI tests).
- Each backend uses the listing's rule in the stats read: PostgreSQL `visible!(4, 5)` with `tx.cutoff()`/`tx.now()`
  and the shared `upper` range; SQLite the new `visible!()` macro (now shared with `outcomes`) with `self.cutoff(now)`
  and `tx.now`; memory `is_visible` with the same `now`. Each case also asserts count == listing.
- Ran `cargo test -p cicada stats` with PostgreSQL: 15 passed (memory, SQLite, PostgreSQL).
- `#[non_exhaustive]` on both structs; `failed: Option<u64>` with `serde(default)`; openapi.json not required,
  nullable. `cicada stats` prints `-`; cli.md regenerated. No struct literals in examples/ or sample-service/.
  CHANGELOG Added and BREAKING lines match. Only generated docs touched. Versions 0.26.0. Merges cleanly onto main.

## Notes

1. `crates/cicada/src/tests/fuzz/compare.rs:1181` is 130 columns (rustfmt skips the `json!` body). Split it.
2. `docs-handoff.md` line 10 (the two test names) is 123 columns.
3. The SQLite failed count filters only by `substr`, not by the `>= / <` range of the other two counts. This matches its
   listing (decision 2) and the result is right.
4. The CLI test does not check JSON `null` for an older server, which the handoff states (true by serde).

## Not checked

The full workspace, the gate evidence, E2E, and a fresh regeneration of openapi.json and cli.md.
