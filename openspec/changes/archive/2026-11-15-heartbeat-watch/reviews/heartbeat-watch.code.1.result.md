Verdict: fix needed

Blocking findings:

1. `crates/cicada-actors/tests/wire_formats.rs:433–450`: the agreed “Schema and decoder boundaries” scenario has no test. Valid samples pass schema and serde; invalid heartbeat states are tested only with serde. Nothing exercises shape-valid, cross-field-invalid heartbeat state accepted by a non-hosting node and subsequently failing handling with cause `state`. Add a virtual-time integration test using the published heartbeat schema, a non-hosting API node and the hosting executor; for example, `status: "alive"` without `armed`. Assert acceptance, then `FailureCause::State` and no heartbeat effects.

2. `crates/cicada-actors/tests/heartbeat_watch.rs:593–619`: “Self target and no-op priority” omits eligible Expire. Its only alive Expire comes from the client and exits before the self-target guard at `src/heartbeat_watch.rs:310`. The builder's impossibility claim does not hold for the public leased-turn API: complete a leased watch turn with `StateReport::Set` containing alive self-target state and `TurnEffects.sends` containing an Expire whose id equals `armed`; completion supplies the watch as sender. Run that eligible timer with the real actor and assert permanent refusal, unchanged state/version and no cancel/send effects. Keep the forged/inactive no-op checks.

Notes: No implementation defect found by inspection. Version 0.18.10, generated-reference scope, example and docs ownership/handoff are consistent with the change. Sample-service hosting is outside the agreed scope.

Not checked: Tests, Docker and gate evidence were not rerun; builder-reported results were inspected. The public completion construction above was verified in source, not executed.
