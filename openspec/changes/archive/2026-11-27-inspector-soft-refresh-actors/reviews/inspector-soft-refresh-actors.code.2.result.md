Verdict: pass (ready to land)

## Round 1 findings

1. Fixed. `run` (`actors.rs:883`) marks only parents outside this run's list. A chosen parent keeps its mark state until its read lands. A failure or deadline marks it (`actors.rs:1208`), and so does one that can no longer be read (`actors.rs:997`). Nothing flickers. A parent past the 20-parent limit is not in `parents`, so it is still marked (`actor_lists.rs:190` covers an unchosen parent).
2. Fixed. `sample_applied` moves only on success (`actors.rs:1256`), and `ticket < sample_applied` drops older answers. An older sample can no longer overwrite a newer successful one. The new test covers the late answer.
3. Fixed. `deleting_a_child_keeps_the_open_parent_and_the_selected_sibling` matches the scenario.

The spec and design equal main. Decision 4 does name only the running refresh. Dropping a queued one uses the shared `Refresh::cancel`, as the other pages do. I accept the claim.

## Notes

- When Retry cancels a running refresh (`actors.rs:744`), chosen parents whose reads were dropped keep no mark. Parents that were not chosen are marked. Before this fix, both kinds were marked. The root rows behave the same way and the design is silent, so this does not block. A note in the handoff or a later proposal would cover it.
- The handoff's mark list leaves out closed rows' cached children.

## Not checked

The GUI, the gate, workspace tests and Clippy. I ran `cargo test -p cicada-inspector`: 116 passed, plus 1 doctest.
