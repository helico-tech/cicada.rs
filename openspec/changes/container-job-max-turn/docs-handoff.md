# Docs handoff for container-job-max-turn

Version: 0.22.0 (the unreleased batch, with `watch-absent-target` and `docker-context`). Refresh the dependency
pins of `docs/how-to/install.md` once it is published.

## What changed

`ContainerJob` has a new associated constant, `const MAX_TURN: Option<Duration> = None`, and `ContainerActor<J>` uses it
as its `Actor::MAX_TURN`. A host sets it on the job to give its container actors a longer turn than the node's default,
so a slow image pull fits one turn without raising `NodeConfig::default_max_turn` (which `--max-turn` /
`CICADA_MAX_TURN` can lower again). The rules of `Actor::MAX_TURN` hold: it wins over the node's default and over
`--max-turn`; `None` keeps the node's default; zero is refused when the type is registered ("MAX_TURN must be
positive"). It is not part of the contract: no hash changes.

A turn past the limit ends `failed` (`timeout`) and is not counted as a runtime error in the container's state: the
poll's own retry policy runs the step again (7 attempts, waiting 1 to 32 s). Every step is safe to repeat; a container
the cut-off step already made is found by its name and not made twice. After the last attempt the container waits for
the next `Check`, `Start`, `Stop` or `Remove`, as for any timed-out step.

Proved by `the_actors_turn_limit_is_the_jobs` (`crates/cicada-containers/src/actor/mod.rs`), and in
`crates/cicada-containers/tests/actor.rs` by `a_create_step_past_the_nodes_default_fits_the_jobs_turn_limit`,
`a_create_step_past_the_jobs_turn_limit_times_out_and_its_retry_makes_no_second_container` and
`a_zero_turn_limit_is_refused_at_registration`. The rustdoc of `ContainerJob::MAX_TURN` has a doc example.

## 1. Set the job's turn limit

**Where.** `docs/guide/containers.md`, `Define a job` (or a short subsection after it).
**Text.** A job can set `const MAX_TURN: Option<Duration>`, the turn limit of its actors. Two turns of the container
actor are long: the create step (a pull of a missing image, the create, the folders and the copies in) and a stop,
which waits up to `stop_grace`. Set the limit above both; otherwise a slow pull, or a stop, ends `failed` (`timeout`)
and is retried. Example, from the rustdoc of `ContainerJob::MAX_TURN`:

```rust
impl ContainerJob for Tests {
    const TYPE: &'static str = "ci.tests";
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(35 * 60)); // a slow pull fits one turn
    type Request = Test;
    // fn spec(..)
}
```

**Caveats.** It wins over `--max-turn` / `CICADA_MAX_TURN`, so an operator cannot lower it from the command line. Zero
is refused at registration. A timed-out step is retried up to 7 times by the poll's retry policy, then the container
waits for the next `Check`, `Start`, `Stop` or `Remove`.

## 2. Rules for hosts

**Where.** `docs/guide/containers.md`, `Rules for hosts`, the bullet `Keep the node's max_turn above an image pull and
the stop grace`.
**Text.** Point it at the job's `MAX_TURN` instead of the node's `max_turn`: "Set the job's `MAX_TURN` above an image
pull and the stop grace, which run inside one turn; without it the node's default applies."

## 3. Operations

**Where.** `docs/operations.md`, the paragraph that says "An image pull and the stop grace run inside one turn: keep
`max_turn` for the type above both".
**Text.** Name `ContainerJob::MAX_TURN` as the way to set that type's limit; the node's `--max-turn` does not change
it once the job sets one.

## Blocking

None.

## Status

Proposed text only; nothing prepared by the docs session yet. Checks: `cargo test -p cicada-containers --all-features`
(169 + 42 + 9 doctests passed), `cargo test -p cicada-examples --test docs` (11 passed), `cargo test --workspace` with
PostgreSQL (3460 passed, 0 failed), clippy and fmt clean.
