# Tasks

Normal lane, additive: in the unreleased batch after 0.23.0 (0.23.1; check the `release:` line of
`python3 scripts/status.py`, builder.md, Version). Follow `design.md`; the specs are `specs/container-actor/spec.md`
and `specs/container-runtime/spec.md`. Run the crate's tests with every feature:
`cargo test -p cicada-containers --all-features`.

- [x] 1. **The read cap.** `CopyLimits::max_read` (default 4 × `max_bytes`, saturating), `with_max_read` (zero refused),
  counted over the content bytes of regular files, kept and skipped, in the shared collector of `copy.rs`, so Docker
  and the fake stop at the same point with one skipped entry for the rest (decision 2). Verify: collector and fake
  tests; `cargo test -p cicada-containers --all-features`.
- [x] 2. **Giving up after three failures.** In the collecting phase, a step message with `failures >= 3` records the
  pending end without a copy, log read or progress read, with `outputs.error` naming the failures and the last error
  (decision 1); the rustdoc of `MAX_TURN` updated. Verify: a test runtime whose `copy_out` sleeps past `MAX_TURN`;
  after three timed-out tries the end is sent and no fourth copy is made.
- [x] 3. **Warnings.** A warning once store errors in a row reach `max_errors` while collecting and while deleting
  outputs; the rustdoc of `max_errors` (decision 3). Verify: a test that one warning is logged.
- [x] 4. **Changelog and docs handoff.** The unreleased section names `max_read` and the three-tries rule under Added
  and
  the dropped small files after a huge one under Changed; the handoff carries the design's Docs section. No edit to
  `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate collect-timeouts --strict`, commit, then
  `python3 scripts/gate.py run`.
