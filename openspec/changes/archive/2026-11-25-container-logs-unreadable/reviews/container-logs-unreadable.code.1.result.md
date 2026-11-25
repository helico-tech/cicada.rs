Verdict: pass

## Blocking findings

None.

## Notes

1. `crates/cicada-containers/src/actor/mod.rs:674`: the end's logs read `(no logs: unsupported: logs of <name>: …)`, the kind included, where design decision 3 writes `(no logs: <message>)`. This matches the existing `(no logs: {e})` format, the spec ("the runtime's message") and the handoff. The builder named it as a departure. Fine as it is.
2. `logs` in the actor: the `Unsupported` arm leaves `errors` as it was rather than resetting it. The end is recorded right after, so this does not matter today. It would matter only if a later change kept polling after a `None`-free return.
3. Gate evidence exists for the pre-rebase head 6d1c54a7, not for cb7558e7. The rebase onto a8d52930 touched only docs (`e2e/` and `cicada-containers` have no commits between them), so the risk is low. The landing gate on main still has to record it.
4. The E2E step uses an `async ||` closure. That is fine with rust-version 1.94 and edition 2024.

## Checked

- Tasks 1-4 are ticked, and their work is in the diff: `map::log_read_failed` is used by both log calls after the 404 check; the rustdoc of `logs`, `logs_since`, `ErrorKind::Unsupported` and `DockerRuntime` is updated; the actor arm is in; there are 3 actor tests, the E2E step and the CHANGELOG entry under `### Changed`.
- Every spec scenario has a test: the 501/500 mapping (unit), "An ended container without logs" (actor), "Log driver none" and "A follower stops" (E2E, `logs_since` created and after the run). "Starting a missing container" and "Copying into a running container" already had tests.
- No contract, wire format or schema change. The `(no logs: …)` wire description still holds. The versions are 0.22.0 in Cargo.toml, both skills and plugin.json.
- No protected docs are in the diff. The handoff's claims match the code (message format, `Method::LogsSince` exists, 404 is `Conflict`). It covers the one stale line in `docs/guide/containers.md:181` that says `Other`, and nothing else in docs/skills/README mentions the log driver.

## Not checked

I ran no tests, E2E or `openspec validate`. I did not check against a live daemon that Docker really answers 501 for a container that was only created; I relied on the builder's pre-rebase E2E run against Docker 29.4.0.
