# Tasks

The numbers in brackets are the decisions of `design.md`.

## 1. The comparison

- [x] 1.1 Write the fourteen operations of the proposal as a test of their own in `crates/cicada-fuzz` [5]. Verify:
      it fails before task 1.2 with the difference at operation 13 that the proposal shows.
- [x] 1.2 `crates/cicada-fuzz/src/compare.rs` [1]: each side keeps the epochs it was handed per address;
      `Side::claimed` applies `epoch_delta` by rank among them, with a number never handed for that address where no
      earlier or later epoch was seen; `close_claim` closes by the epoch that was sent. Add the rule to the header of
      the file. Verify: the test of 1.1 passes, and new tests pass for the scenario "Two instances are due at the
      same instant" with a commit, a renew and a release, each with `epoch_delta` `-1` and `1`.
- [x] 1.3 Add the `cc` line of the proposal to `crates/cicada-fuzz/tests/backends.proptest-regressions`, in the
      format of `parsers.proptest-regressions` [5]. Verify: `cargo test -p cicada-fuzz --test backends backends_agree`
      passes with the line, and fails with it when the change of 1.2 is taken out (say in the report how you
      checked).
- [x] 1.4 Run `generated_lists_reach_deep_states` and `generated_lists_reach_every_operation`. Verify: both pass; if
      a floor had to change, the report says which and why.
- [ ] 1.5 A long run: `PROPTEST_CASES=20000 cargo test -p cicada-fuzz --test backends`. Verify: it passes. A new
      difference is not fixed here and its `cc` line is not committed: report it with the line and the shrunk list.

## 2. Changelog and checks

- [ ] 2.1 `CHANGELOG.md`: one entry under Fixed for the comparison of `cicada-fuzz`. Verify: the section says that
      no library crate changed.
- [ ] 2.2 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
      `openspec validate fix-fuzz-claim-order --strict`. Verify: all pass.
