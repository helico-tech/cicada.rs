Verdict: fix needed

Blocking findings

1. `crates/cicada/src/api/leases.rs:141–143` (with `#[serde(remote = "Self")]`, e.g. line 271): the object-only check applies only to the trait implementation. Serde's remote derive also generates a **public inherent** `deserialize` method on each public view, and ordinary `Type::deserialize(...)` resolves to that method, bypassing the wrapper. Consequently `ControlView::deserialize(&mut serde_json::Deserializer::from_str(r#"["delete","c/d"]"#))` accepts a delete, and `TurnEffectsView::deserialize` accepts `[]`, contradicting these views' documented object-only decoding. Confirmed against the builder's artifacts with `/tmp/cicada-review-round2.rs`; `serde_json::from_str` correctly rejects the same inputs. Keep the unchecked derived decoder private and make normal public deserialization consistently enforce object forms; add regressions exercising direct `Type::deserialize` as well as generic serde decoding.

Notes

- The original HTTP route bypass is fixed, with API and route regressions checking refusal before mutation. This finding concerns the newly exposed public decoder path, not an HTTP-route bypass.
- The worker tests now use paused time and in-flight barriers; the remaining SQL cases are framing smoke tests. The handler-start synchronization and guide clarification are sound.

Not checked: no suite rerun, release checks, or Docker E2E. Reviewed only the round-2 diff and necessary context. Assessed builder-reported workspace/PostgreSQL, repeated runtime tests, fmt and clippy results as reported; ran only the focused decoder experiment above.
