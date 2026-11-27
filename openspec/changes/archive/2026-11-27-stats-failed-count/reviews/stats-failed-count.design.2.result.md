Verdict: ready for tasks

Round 1 blocking finding is fixed. Decision 1 now states the listing's rule without `since`: visible_until alone decides when set, else store retention. This matches `visible!` (postgres), the SQLite CASE and `is_visible` (memory). The spec has a "Hidden by its keep policy" scenario for Keep::Remove, and the design's Tests list has the conformance case plus the past-retention and visible_until cases.

Notes from round 1 are handled: PG heap-fetch cost, "same read" wording (store clock, not a snapshot), `#[non_exhaustive]`, the crate list (cicada-axum dropped, StatsView in cicada's api module), TUI out of scope, no ADR.

New gaps: none blocking.

Notes:
1. The tasks should add the conformance cases for Keep::For shorter than retention and Keep::Last after trimming. Design names them, but only Keep::Remove has a scenario.
2. non_exhaustive means in-crate tests and cicada-client can still build the structs, but external crates cannot. Tasks should check examples/ and sample-service/ for literals.

Not checked: tests, query plans.
