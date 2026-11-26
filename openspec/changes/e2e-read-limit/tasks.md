# Tasks

Fast lane, tests only: no version change (the 0.23.1 release carries it).

- [x] 1. **The E2E check.** In `copy_out_keeps_within_its_limits`, the byte-limit check uses
  `CopyLimits::new(10, 100).with_max_read(10_000)`, and a new check with `CopyLimits::new(10, 100)` (read limit 400)
  finds a skipped entry whose reason names the read limit. Verify: `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`.
- [ ] 2. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, then the 0.23.1
  release runs the gate with E2E.
