# Tasks

## 1. Crate and replay

- [x] 1.1 Add `crates/cicada-fuzz` to the workspace, with `proptest` as a workspace dev dependency. Verify: `cargo test -p cicada-fuzz` runs with no tests failing, with no database or Docker.
- [x] 1.2 Add `biased;` to the `select!` calls at `executor.rs:350`, `system.rs:433` and `system.rs:622`. Verify: `cargo test -p cicada-test` and `cargo test -p cicada` pass unchanged.
- [x] 1.3 Add the crate to the tables in `docs/architecture.md` and `AGENTS.md`, with the `PROPTEST_CASES` command for a long run. Verify: `cargo test -p cicada-examples --doc` passes and the documented command runs as written.

## 2. Input fuzz

- [x] 2.1 Apply `fix-duration-overflow` and merge it first, as the duration test below is expected to fail without it. Verify: its unit test passes and its overflow input is refused. Done as its own commit, ahead of the harness on the same branch, as the owner agreed at the review.
- [x] 2.2 Add `tests/parsers.rs`: no panic on random and on near-valid text, for addresses, percent-encoding, durations, instants and page cursors. Verify: passes at `PROPTEST_CASES=100000`.
- [x] 2.3 Add the round-trip properties for addresses, durations, instants and page cursors. Verify: passes at `PROPTEST_CASES=100000`.
- [x] 2.4 Add the no-panic property for the HTTP request bodies in `cicada::api`. Verify: passes at `PROPTEST_CASES=100000`.

## 3. Backend comparison

- [x] 3.1 Add the operation generator, covering every `Store` method except the node display rows, with small pools of addresses and ids. Verify: a test asserts that generated lists reach every operation.
- [x] 3.2 Add the driver that runs one list on `InMemoryStore` and `SqliteStore` on one virtual clock. Verify: a test shows that a deliberately changed answer from one backend is reported as a difference.
- [x] 3.3 Add the comparison rules: `seq` and `epoch` by rank, errors by kind, paging with each backend's own cursors. Verify: unit tests for each rule, in both the accepted and the refused case.
- [x] 3.4 Add the end-of-list comparison of actors, pending messages and retained outcomes. Verify: a seeded difference in final state is reported.
- [x] 3.5 Set the case count for `cargo test` from measured time. Verify: the test takes under 30 seconds in a normal `cargo test` run, and passes at `PROPTEST_CASES=5000`.
- [x] 3.6 Write up each difference found: a named rule with a reason, or a change with only a `proposal.md`. Verify: `openspec list` shows the findings and no difference is left unexplained.

## 4. Review checkpoint

- [x] 4.1 Stop and review with the owner: what A and B found, CI time, and whether the simulation still looks right. Verify: the owner has said to continue, and the outcome is written into this file under the task.
  - Outcome: the owner said to continue, unchanged. A found the duration overflow (fixed, saved case kept). B found one difference, the claim order among instances, now a named rule. The fuzz crate adds about 6 seconds to `cargo test`.

## 5. Fault layer

- [ ] 5.1 Add `FaultStore`, with every `Store` method going through one gate. Verify: a test on `InMemoryStore` checks that with no fault rules each method gives the inner store's answer.
- [ ] 5.2 Add the Before, After, Delay and Dead faults and per-node rules keyed by operation and call count. Verify: one test per fault, on paused time; After leaves the commit applied and returns `Unavailable`.
- [ ] 5.3 Add the event log, recording the inner store's real answers and the handler starts and ends. Verify: a test sees a lost-acknowledgement commit in the log.

## 6. Node simulation

- [ ] 6.1 Add `Scenario`, its generator and the one script actor. Verify: a test runs a scenario where each script action happens at least once.
- [ ] 6.2 Add the runner: several `ActorSystem`s on `#[tokio::test(start_paused = true)]` and `InMemoryStore`, each with its own `FaultStore`, then heal and drain. It does not use `TestActors`, which has one node and owns its store. Verify: a scenario with no faults drains with every message `done`.
- [ ] 6.3 Add the replay check. Verify: a test runs one scenario with faults twice and the event logs are equal.
- [ ] 6.4 Add the checks G1, G3, G4, G5, G6 and S8. Verify: each check has a unit test on a hand-written bad event log that makes it fail.
- [ ] 6.5 Add `tests/simulation.rs` and set the case count for `cargo test` from measured time. Verify: under 30 seconds in a normal run.
- [ ] 6.6 Prove the harness catches real bugs: temporarily break the fencing check in `InMemoryStore`'s `commit`, and separately its head-of-mailbox check in `claim_due`. Verify: the simulation fails on G1 or G3, and on G5, with a short shrunk case; both breaks are reverted.
- [ ] 6.7 Run long and write up each failure as a change with only a `proposal.md`, holding the shrunk case. Verify: `PROPTEST_CASES=20000 cargo test -p cicada-fuzz --test simulation` passes, or each failure has its change.

## 7. Final checks

- [ ] 7.1 Fix every finding that blocks green in its own change, with its saved case committed there. Verify: no open finding has a saved case failing on `main`.
- [ ] 7.2 Run the full checks. Verify: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass, and `openspec validate add-fuzz-harness --strict` passes.

## Workflow follow-up

- Archive the change with `/opsx:archive` once it is merged.
