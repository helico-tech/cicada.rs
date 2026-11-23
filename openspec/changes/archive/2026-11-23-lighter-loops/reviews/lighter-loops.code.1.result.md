Verdict: fix needed

## Blocking

1. `scripts/status.py:71-80` (`landed`) and `:83-101` (`release_line`): every change archived since the last tag
   counts as part of the batch, including a process or docs-only change that lands with `Version: none` and no
   changelog entry, such as `lighter-loops` itself. Such a change has no `## Lane` section, so it counts as a normal
   lane change. That breaks decision 5: a batch of fast-lane changes alone must wait. The count is wrong too.
   To see it: in a temporary repo with tag `v0.1.0`, archive a change with no Lane section and no changelog entry,
   then add `## 0.1.1 — unreleased` and archive a `Lane: fast` change. The branch's `status.py` prints
   `release:  0.1.1 unreleased, 2 changes landed; release now: no change is ready or in build`. It should wait.
   The change loop goes to Release on that line, and with a standing yes it publishes.
   Fixed means: changes that are not in the release are left out of the batch, for example those whose archived
   `docs-handoff.md` says `Version: none`. A `test_status.py` case covers it.

## Notes

1. `openspec/config.yaml` context still says tasks.md is written "after a review". Fast-lane changes have no design
   review, so add "(normal lane)" there.
2. `release_line`: when there is an `unreleased` heading but nothing landed, `all()` over the empty batch prints
   "no landed change outside the fast lane". That is true but misleading. Saying "no change landed" would be clearer.
3. `take ID` silently ignores `--wait`. The docstring says so; an argparse error would be stricter.

## Not checked

- I did not run the test suites or the gate. `gate.py status eaf1a0f` shows a full pass recorded. E2E was not
  needed, because no crate changed.
- I did not run `openspec validate`.
- I read the change-loop skill against decisions 2–15 once and found no gaps beyond the above. I did not check every
  sentence of the builder brief line by line.
- The protected files `AGENTS.md`, `docs/how-to/releasing.md` and `docs/maintaining/maintenance.md` equal `6e86dc0`
  (the diff is empty) and contain only prose. No leftover `design-loop` or `build-loop` references remain.
- The tasks have no spec deltas (`skip_specs: true`), so there were no scenarios to map to tests.
