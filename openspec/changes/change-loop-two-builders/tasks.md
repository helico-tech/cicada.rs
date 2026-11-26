# Tasks

Normal lane, process only: version `none` (no crate change, no release). Follow `design.md` and ADR 0073
(`docs/adr/0073-two-builders-in-the-change-loop.md`); the spec is `specs/change-loop/spec.md` (a new capability).

- [x] 1. **status.py.** Reads the `Crates:` line of each proposal's Impact; the build rule of decision 1 (slots filled
  by
  changes in build, a free slot to the first ready change whose crates meet none in build, a change without the line
  only alone); up to two names on `build:`; a problem for two changes in build sharing a crate; `Order` resolved through
  archived changes (decision 6). Verify: the status tests of the design.
- [x] 2. **review.py.** `--built-by`, model families from names (`claude`, `gpt`; `codex` and `openai` as `gpt`), `--by`
  required with a family for code reviews, `take ID` refusing and `take` skipping the builder's family,
  `--same-family REASON` recorded, `list` showing builder and reviewer (decisions 4 and 5). Verify: the review tests.
- [ ] 3. **The loop's skill and the config.** `.claude/skills/change-loop/SKILL.md` and `builder.md`: two builders, the
  Codex call (decision 3), reviews by the other model with the fallback, landing in any order, the gate retry;
  `openspec/config.yaml`: the `Crates:` rule and the throughput line (decision 1); a `Crates:` line in every open
  proposal's Impact. Verify: `python3 scripts/status.py` names the expected builds and no problem.
- [ ] 4. **Docs handoff.** `docs-handoff.md` gives `AGENTS.md` ("Specs and changes", Flow) and the change-loop section
  of
  `docs/how-to/releasing.md` the new rules; nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten
  docs.
- [ ] 5. **Finishing checks.** `python3 -m unittest discover -s scripts -p 'test_*.py'`,
  `openspec validate change-loop-two-builders --strict`, commit, then `python3 scripts/gate.py run` (wait for the lock
  in
  short tries; the run as its own background command with a 2-hour timeout).
