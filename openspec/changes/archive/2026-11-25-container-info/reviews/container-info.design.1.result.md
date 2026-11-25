Verdict: ready for tasks

## Blocking findings

None. One behaviour, about 3 tasks. Checked against the code: `Method` and `Call` are `#[non_exhaustive]`, `docker()` retries a failed agreement, `kind_of` maps IO, timeouts and a missing socket to `DaemonUnavailable`, bollard 0.21.1 has `version()` with `Arch`/`Os` and a 120 s default timeout. `openspec validate` passes. No clash with `container-registry-auth`.

## Notes

1. **A daemon that answers with a 5xx.** `kind_of` maps every 5xx to `Other`, and the design's sketch tells a host to stop on `Other`. A Docker Desktop engine that is stopped or starting can answer through its socket proxy with a 500, so Factory would stop instead of skipping `Here` for that round. Say in the rustdoc of `info` which kind a 5xx gives (or see Q1).
2. **A scenario with no test.** container-runtime, "A runtime that answers" (an arm64 Linux machine) maps to no test in design.md Tests. Rewrite it for the fake (`set_info`) or the E2E, or drop it.
3. **The docs target.** design.md Docs says "the container guide", but `docs/guide/` has no container guide, and earlier container handoffs name none. The handoff should leave the place to the docs session rather than name a guide that does not exist.
4. **The name.** `info` reads like Docker's `/info`, which the design turns down (it uses `/version`). The rustdoc should name `/version`. `platform()` would be clearer, but this is optional.
5. **An old daemon.** A daemon below API 1.44 gives `Other` from `docker()`, not `DaemonUnavailable`. The spec sentence "cannot be reached" allows this, but say it in the rustdoc. The first call reads `/version` twice (agreement, then the call). That is harmless.

## Questions for the owner

- Q1: Should a 5xx from `/version` count as "Docker does not answer" (`DaemonUnavailable`) for `info` only, or stay `Other` like every other call? My pick: keep `Other` and document it, so the error kinds stay the same across calls.

## Not checked

I did not build anything or run tests. I did not check how Docker Desktop or OrbStack really answer while the engine is stopped, or whether `/version` `Arch` matches image platforms on 32-bit userlands.
