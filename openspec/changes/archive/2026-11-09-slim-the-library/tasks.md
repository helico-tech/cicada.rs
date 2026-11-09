# Tasks

Each task is verified by `cargo build --workspace --all-targets` and the tests of the crates it touches; the last
section runs everything.

## 1. Small cuts

- [x] 1.1 C11: remove the changes `docker-events-watcher`, `console-container-logs`, `several-docker-hosts`
- [x] 1.2 C9: remove the unused public items (`ActorSystem::hosted_templates` stays: `TestActors` uses it)
- [x] 1.3 C10: remove `differential.rs` and the other duplicate tests

## 2. Core API

- [x] 2.1 C1: remove `Unhandled::Custom`, `on_unhandled`, `UnhandledDecision`
- [x] 2.2 C12: plugins observe, plus the `on_claim` veto; remove `before_commit`, hook sends, `set_outcome`, reserved
      headers, per-type installs
- [x] 2.3 H4: `HookSend` is gone, `SendOptions` only
- [x] 2.4 H5: `NodeConfig` and `.config()` only
- [x] 2.5 H6: remove `ActorType::max_turn()`
- [x] 2.6 H7: `install` only
- [x] 2.7 H8: one function builds a contract

## 3. Stores

- [x] 3.1 C13: remove the message chain lookup; `caused_by` stays
- [x] 3.2 C5: remove `retention()`, `type_def()`, `outcome()`, `resolve()` from the `Store` trait
- [x] 3.3 C6: store settings nobody sets become constants
- [x] 3.4 C7: remove `PostgresConnection` and `connect_as`; remove the change `redact-connect-options-url`
- [x] 3.5 H9: backend-neutral logic once, in `store::support`

## 4. Wire

- [x] 4.1 C8: remove `GET /types?uri=`, `TypeMatchView`, `ActorClient::type_of`
- [x] 4.2 H1: ISO-8601 durations only, negative refused; the short form moves to `sample-service`
- [x] 4.3 H2: one policy decoder
- [x] 4.4 H3: characters, byte order, a plain lock key, Rust hook names
- [x] 4.5 H11: `api::Params` becomes a `BTreeMap`

## 5. Extras

- [x] 5.1 C3: remove `ActorOperations` and the `client` feature
- [x] 5.2 C4: remove `Timer`
- [x] 5.3 H10: dead-letter ids from `sha2`; `md-5` leaves the workspace

## 6. Docs and release

- [x] 6.1 A2: README recipe for a timer
- [x] 6.2 README, `docs/architecture.md`, `docs/operations.md` match the code
- [x] 6.3 ADR 0012 for the wire and API changes
- [x] 6.4 A1: `CHANGELOG.md`, version 0.2.0
- [x] 6.5 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
      `cargo test -p cicada-examples --doc`
- [x] 6.6 A4: `ClaimContext::send_headers`; `ActorTelemetry` sends trace headers on its own
