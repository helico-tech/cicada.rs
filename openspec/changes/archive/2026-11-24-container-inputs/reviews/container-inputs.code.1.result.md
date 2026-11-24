Verdict: pass

## Blocking findings

None. Tasks 1–5 are ticked and in the diff. Every spec scenario has a test in `tests/actor.rs`. The version is 0.20.9 everywhere, CHANGELOG is updated, no protected docs are touched, and the docs handoff matches the code.

Replay safety holds. `create` keeps an existing container, so a replayed step finds it `Running` or ended, skips the copy, and `start` does nothing. A container still `Created` gets the inputs again, which is safe. If a run whose claim lapsed hits `Conflict` while another run starts the container, it counts an error, and its commit is refused anyway. Error counting is right: each failure adds one, `max_errors` ends the container `failed`, and a start resets the count to 0.

## Notes

1. `refused` (`src/actor/mod.rs:~598`): every `Other` from `make_dirs` ends the container `failed` at once, as the spec says. But Docker's `kind_of` maps every 500 to `Other` (a failed stat or upload, and a 404 on upload after a removal). So a passing daemon error ends a Docker job for good, while the same error from `copy_in` backs off. The design gave only the trait's default as the reason. Suggest a line in `container-limits-and-kinds`: mark "not supported" so a Docker `Other` backs off.
2. `src/actor/mod.rs:280`: a re-wrapped rustdoc line is 140 columns. rustfmt does not wrap comments, so it passes; please re-wrap it to 120.
3. The replay test starts the container outside the actor instead of losing a real commit. That is fine for the scenario. No test covers a replay on a `Created` container that already has some inputs.

## Not checked

I ran no tests, clippy, fmt or gate; I relied on the builder's report and its gate evidence. I read Docker behaviour from the code only, with no E2E run.
