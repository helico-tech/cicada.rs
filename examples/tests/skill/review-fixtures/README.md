# Review fixtures

Small, test-only inputs for the review cases of `actor-design-evals.json`. Each folder is a fragment of an imagined
consumer project: Rust files written against Cicada's public API and design documents. They are not compiled, not
packaged in a release archive, and not examples to copy; some hold deliberate defects for the review to find.
`examples/tests/skill.rs` checks that every fixture a case names exists here.
