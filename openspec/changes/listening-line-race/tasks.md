# Tasks

Fast lane, tests only. Version: none (no crate's behaviour, API or output changes; no changelog entry).

- [ ] 1. **Complete lines only.** In `start_as` of `sample-service/tests/sample_service.rs` and `serve` of
  `crates/cicada-app/tests/common/mod.rs`, look for the listening line only among lines that end in a newline: take
  the output up to its last `\n` and search those lines, so a last line still being written is never read. Keep the
  rest of each loop (the end check, the 20 s limit). Do not change `serve`, `node.rs` or the line it writes. The
  helper at `sample_service.rs` line ~434 reads a pipe with `BufRead::lines`, which already waits for the newline:
  leave it. Verify: `cargo test -p cicada-app --all-features` and `cargo test -p cicada-sample-service`, each run three times without a failure.
- [ ] 2. **Docs handoff.** Write `docs-handoff.md` from the template in `builder.md`: `Version: none`, nothing for
  users, nothing blocks.
- [ ] 3. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `openspec validate listening-line-race --strict`, commit, then `python3 scripts/gate.py run --e2e` (the change
  touches `sample-service/`).
