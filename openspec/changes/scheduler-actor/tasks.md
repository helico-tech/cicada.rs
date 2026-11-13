# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `value-depth-limit` has landed when this is built (0.16.0); this change is 0.16.1, as it
breaks nothing. Runtime tests run on virtual time with `#[tokio::test(start_paused = true)]` and `TestActors`, in the
tests of `cicada-actors`, as those of the topic and the supervisor do.

## 1. The actor

- [x] 1.1 The types [2, 3]: a module `scheduler` in `cicada-actors` with `Scheduler` (`named`, type name
      `cicada.scheduler`), `SchedulerState` with its public fields and
      `SchedulerState::new(schedule, &target, message)`, `SchedulerMsg` (`Start`, `Run { slot }`,
      `Update { schedule, target, message, retry }`, `Tick { slot }`) and the unit struct `SchedulerActor`; exported
      from the crate root and named in the table of `src/lib.rs`. Doc comments on every public item. Verify: the JSON
      of the state and of an `Update` in `design.md` decode to the values they name; a doctest of
      `SchedulerState::new`, and a `compile_fail` doctest for a message of another family.
- [x] 1.2 The chain [3, 4, 5, 7]: `Start` cancels the armed tick and arms one at `schedule.next(now)`; a `Tick` counts
      only from the scheduler itself and as the armed one, arms the next at `schedule.next(max(now, slot))` and sends
      the scheduler `Run { slot }`; a `Run` sends the stored message with `send_raw`, with the scheduler's `retry` and
      the header `cicada-slot` when it has a slot, then adds 1 to `runs` and sets `lastRun`. `Tick` and the `Run` a
      tick sends carry the crate's shared retry policy. Verify: the scenarios of the requirements "A scheduler keeps a
      schedule, a target and a message", "A scheduler sends its message to its target on each slot", "The target learns
      its slot" (the header's text checked character by character), "A scheduler skips the slots it missed" (the
      early tick through `ActorSystem::deliver_now`) and "The ticks of a scheduler are its own".
- [x] 1.3 The operator's messages [3]: a `Run` without a slot, and `Update`. Verify: the scenarios of the requirements
      "An operator runs a schedule now" and "An operator edits a schedule".
- [x] 1.4 Failures [6]: a target that is gone (cause `commit`), a message it does not take (cause `refused`), a read of
      the store that fails (retried, as `tests/store_failure.rs` does for the topic), and a chain that ends. Verify: the
      scenarios of the requirements "A failed delivery does not end the chain" (the retry through
      `ActorSystem::retry_message`, and a redrive through `DeadLettersActor` that watches the scheduler) and "A start or
      a new schedule arms a chain again" (the tick that fails for good through `claim_and_abandon` up to the claim
      limit, the purge through `ActorSystem::purge_mailbox`).
- [x] 1.5 How deep [8]: a stored message whose payload is 62 levels deep is accepted, one of 63 levels is refused at
      the create of the scheduler and at the send of an `Update`. Verify: tests at `ActorSystem`.

## 2. Wire formats and the sample service

- [x] 2.1 The contract [2, 3]: `docs/reference/contracts/cicada.scheduler.json`, written with
      `CICADA_UPDATE_CONTRACTS=true`; `tests/wire_formats.rs` checks it, the JSON that the state and the messages are
      written as, and sample checks that the registered schemas and the decoders take what `SchedulerState::new` and
      the actor write (ADR 0026). Verify: `cargo test -p cicada-actors --test wire_formats` passes without the
      variable.
- [x] 2.2 The sample service [9]: its catalog hosts `SchedulerActor`, with no singleton; its contract snapshot in
      `sample-service/tests/contracts/` and `tests/contract-texts.json`; a state sample of an armed daily scheduler in
      `tests/states/`, as `cron` has. Verify: `cargo test -p cicada-sample-service` passes.

## 3. Docs

- [x] 3.1 The actors guide [9]: the section "Schedules" becomes "Scheduler", with doctests: the actor and its state,
      its messages, the header `cicada-slot` and its form, the retry policy of the delivered message (one attempt for
      none), a client's `Run` without a slot, two deliveries close together after a `Run` before a slot, a target that
      is gone (pause or delete the scheduler; the cost for the dead letters), the ways a chain ends and `Start` as the
      fix, a scheduler that runs itself, how deep (62 levels), and `Schedule` with the link to the cron example as the
      chain for host work. The guide's two tables and its count of the types name the scheduler. Also
      `docs/architecture.md` (the row of `cicada-actors`), `AGENTS.md` (the map), `README.md` (the table of actors).
      Verify: `cargo test -p cicada-examples --doc` and `cargo test -p cicada-examples --test docs` pass, and every
      risk of `design.md` that names the guide is in it.
- [x] 3.2 `CHANGELOG.md`, a section `0.16.1 — unreleased` that names the new actor and that nothing breaks; the version
      `0.16.1` in `Cargo.toml`, `Cargo.lock` updated by `cargo check`, and `docs/openapi.json` regenerated with
      `CICADA_UPDATE_OPENAPI=true`. Verify: the versions agree and the OpenAPI test passes without the variable.

## 4. Checks

- [x] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, `cargo check -p cicada-actors`, `openspec validate scheduler-actor --strict` and
      `openspec validate --specs --strict`. Verify: all pass.
- [x] 4.2 The end-to-end suite with `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`, which starts the sample
      service with the new type on PostgreSQL and SQLite. It needs Docker. Verify: it passes; if Docker cannot be
      started, leave this task open and say so in the report.
