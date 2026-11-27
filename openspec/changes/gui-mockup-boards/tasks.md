# Tasks

Normal lane, no version bump (process only). Follow `design.md`; the spec is `specs/change-loop/spec.md`.

- [ ] 1. **The change-loop skill.** Describe: a GUI change names each screen with its board or "no board", copies its
  boards into `mockup/` with `SOURCE.md`, has "Departures from the board" (additions included), names the board in its
  building task, and starts its manual list with "next to board X, light and dark"; the builder's prompt names the
  boards; Land reads the live boards again and waits on a changed one (decisions 1, 2 and 4).
- [ ] 2. **The builder's brief.** `builder.md`: read the boards before building; never depart without asking; report
  departures (decision 2).
- [ ] 3. **The review briefs.** `review-loop/design.md`: every screen named, every board element in the design or under
  Departures, every addition under Departures. `review-loop/code.md`: compare the screen's code with the board in both
  directions and block on an unagreed departure (decision 3).
- [ ] 4. **The OpenSpec rules.** `openspec/config.yaml`: the design and tasks rules for GUI changes (decision 5).
- [ ] 5. **Checks.** `cargo test -p cicada-examples --test skill` and `--test docs`, `python3 -m unittest discover -s
  scripts -p 'test_*.py'`, `openspec validate gui-mockup-boards --strict`, commit. No `docs-handoff.md` content beyond
  "none".
