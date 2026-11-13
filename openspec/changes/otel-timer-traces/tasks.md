## Implementation

- [x] 1. Add public `observer::Sends` and `TurnStart::add_header_for`, keeping `add_send_header` equivalent to All.
  Carry the two maps through Observed, TurnInfo and the common typed/raw send-header merge. Classify by self-address
  equality and requested At/After, with no clock or actor-type checks. Extend virtual-time observer and send-header
  tests for every send path, zero/past/clamped delivery, delayed sends to other actors, call/observer precedence,
  explicit headers, per-kind capacity/validation and panic isolation. Run `cargo test -p cicada --lib tests::runtime::observer`,
  `cargo test -p cicada-test --test send_headers`, and `cargo check -p cicada` without features.
- [x] 2. Change ActorTelemetry to inject ordinary headers for Others and prefixed headers for Timers, and extract
  independent parent/link contexts with a prefix-stripping reader and one link before span start. Keep metrics,
  outcomes and baggage behaviour unchanged. Extend `crates/cicada-otel/tests/actor_telemetry.rs` on virtual-time
  TestActors with SDK in-memory exporters: assert parent/trace/link ids across recurring timers, immediate self-sends,
  delayed other sends, explicit and all-zero parents, explicit link fields, case aliases/duplicates, malformed or
  omitted fields, header limits, custom/composite propagators, ambient context, policy retries/lost claims, operator
  retry/deliver-now, and legacy headers. Retain no-observer and ordinary-chain regression coverage. Add a dev dependency
  on cicada-actors for actual flow sleep (including zero) and scheduler trace-boundary tests; verify redrive/notification
  header behaviour using existing fixtures where possible. Run `cargo test -p cicada-otel` and affected actor tests.
- [x] 3. Document the additive observer API and ADR 0032 header convention in crate docs, architecture's observer and
  telemetry sections, and the core guide with a compiling header-selection example. Document flow-sleep and scheduler
  trace boundaries in the actors guide, per-attempt roots, explicit parent overrides, best-effort links, sampler
  consequences and old queued messages. Update changelog/version following the release process; no actor contracts,
  schema or HTTP fields change. Run `cargo test -p cicada-examples --doc`, `cargo test -p cicada-examples --test docs`,
  and the affected crate doctests; regenerate a version-bearing reference only if its existing drift check requires it.

## Finishing checks

- [ ] 4. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate otel-timer-traces --strict`; record results and any unavailable optional checks.
