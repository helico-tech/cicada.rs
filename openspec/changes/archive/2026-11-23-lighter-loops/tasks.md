# Tasks

This change is built by the current `/build-loop`; the change loop it adds takes over once it has landed. It changes
no crate, so it lands without a version bump or release (design, decision 6). ADR 0055 and its row are written.

- [x] 1. **The change loop.** Write `.claude/skills/change-loop/SKILL.md` from `design-loop/SKILL.md` and
  `build-loop/SKILL.md`, by decisions 2 to 7, 10 and 12 to 15: Start (fetch, `status.py`, `review.py list`, worktrees),
  Describe (normal and fast lane, the `## Lane` section, the E2E rule of decision 8 in the tasks), Build (the builder
  subagent in a worktree, in the background; describing the next change meanwhile; at most two ready), Review (request,
  then a fresh reviewer subagent, Opus at default effort, with the id), Blocked on docs (as now), Present, Land (merge,
  archive with `--skip-specs` for a change that has `skip_specs`, full gate run, push `main`, no release), Release
  (decision 5, `Release: now`, decision 15) and Rules. Move `build-loop/builder.md` into `change-loop/` with `git mv`
  and change it: `gate.py run` without `--e2e` unless the tasks ask for it, the version rule of decision 7. Remove
  `design-loop/` and `build-loop/`. Verify: `grep -rn "design-loop\|build-loop" .claude scripts openspec/config.yaml`
  finds only what the docs handoff lists; read the skill once against each decision and note the mapping in the
  report.
- [x] 2. **Reviews by id.** `scripts/review.py take <id>`: an optional id that claims exactly that open request and
  fails with a clear message when it is taken, done or missing; without an id it behaves as now. Update the module
  docstring. `review-loop/SKILL.md` says it is for another harness and that the change loop starts its own reviewers;
  `review-loop/code.md` and `design.md` name the change loop instead of the design and build loops, and `code.md`
  reads the proposal and tasks of a fast-lane change, which has no design or specs. Verify: a new
  `scripts/test_review.py` (temporary directory) for take by id, take of a taken id, a missing id and take without an
  id; `python3 -m unittest discover -s scripts -p 'test_review.py'`.
- [x] 3. **The release line.** `scripts/status.py` prints `release:` (decision 9): the version of an `unreleased` top
  heading of `CHANGELOG.md` and how many changes were archived since the last tag, or `release: nothing unreleased`;
  and it says to release when no change is ready or in build and a landed change outside the fast lane, or one with
  `Release: now`, waits. It reads `Lane: fast` and `Release: now` from the `## Lane` section of each archived
  proposal. Update its docstring for the change loop. Verify: a new `scripts/test_status.py` (temporary repository)
  for nothing unreleased, an unreleased batch with a ready change, a batch of fast-lane changes only, a batch with a
  normal change, and `Release: now`; `python3 -m unittest discover -s scripts -p 'test_status.py'`; `python3
  scripts/status.py` on this repository still prints the line of changes with no problems.
- [x] 4. **OpenSpec rules.** `openspec/config.yaml`: the context names the change loop (ADR 0055); the proposal rules
  add the optional `## Lane` section with `Lane: fast` and `Release: now` and the closed fast-lane list; a fast-lane
  change has `proposal.md`, `tasks.md`, `docs-handoff.md` and `.openspec.yaml` with `skip_specs: true`; the task rules
  add the E2E rule of decision 8. Verify: `openspec validate lighter-loops --strict` and `openspec validate
  dependency-advisories` give the same result as before the edit.
- [x] 5. **Docs handoff.** Write `docs-handoff.md` by the template in `builder.md`, with the items of the design's
  "Docs it breaks". The moved `builder.md` link in `AGENTS.md` is Blocking: report `Docs: blocked` with that path, and
  integrate the docs session's prepared `AGENTS.md` when the build loop sends it. Verify: after integration,
  `cargo test -p cicada-examples --test docs` passes.
- [x] 6. **Finishing checks.** `python3 -m unittest discover -s scripts -p 'test_*.py'`, `cargo test -p
  cicada-examples --test docs`, `cargo test -p cicada-examples --test skill`, `openspec validate lighter-loops
  --strict`, then commit and run `python3 scripts/gate.py run` (the full policy; no E2E, as no crate changes).
