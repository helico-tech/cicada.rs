Verdict: pass

## Blocking findings

None.

All five tasks are ticked, and their work is in the diff. Each of the four spec scenarios has a test in
`crates/cicada-containers/src/orphans/tests.rs`, and the extra tests listed in the design are there too (missing in one round only,
a failed actor read, skipped names, zero interval, stop). The round logic matches decisions 1-3: prune to the listed names,
`missing` then `reported`, a failed list or read changes nothing, and nothing is removed. The design's claim that the actor
is saved before its container is true: the first poll, a later turn, creates the container (`actor/mod.rs:703`). The version is 0.23.1 in
`Cargo.toml`, both skills, `plugin.json` and `openapi.json`. The CHANGELOG has an unreleased section. The only protected doc
in the diff is the generated `openapi.json`. The handoff is true to the code: nine tests, two doctests, and the caveats
match the rustdoc. The example matches `App::task`/`Stopping::wait` and `DockerRuntime::connect`.

## Notes

1. `orphans/mod.rs`, the "Rounds" rustdoc says "Each interval, from the start of `run`". In fact `run_on` sets
   `next_round = Instant::now() + interval` after each round ends, so the rounds drift by however long a round takes. Either fix
   the wording ("an interval after each round ends") or schedule from the previous due time.
2. The Limits section names two stores that share a prefix. Two job types with one prefix would also see each other's
   containers as orphans. Their names already collide, so this setup is unsupported anyway, but one line would help.
3. `a_name_without_the_prefix_is_skipped` proves the skip but not the "one warning". That is fine, just nice to have.
4. Decision 4 of `design.md` still says "can ship in 0.23.0". It is stale but does no harm.

## Not checked

I did not run any tests, clippy, fmt or the gate. I did not check the gate evidence recorded for 164a78b6. The design and handoff
`App::task` example was checked only by reading it. It is not compiled anywhere yet: the guide block will be `no_run` from the docs session.
