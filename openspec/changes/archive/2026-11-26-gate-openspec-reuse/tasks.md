# Tasks

Normal lane, process only: version `none` (no crate changes; no release). Follow `design.md` and ADR 0070
(`docs/adr/0070-openspec-files-reuse-a-full-pass.md`); the spec is `specs/local-gate/spec.md` (a new capability).

- [x] 1. **The allowlist.** `"any_under": ["openspec/"]` in `scripts/gate-policy.json`, `openspec/` dropped from
  `markdown_under`; `parse_policy` accepts the key; `allowed_path` accepts paths under `any_under` whatever their
  extension, `except_under` still winning; `classify` unchanged (mode `100644` only) (decisions 1 and 2). Verify:
  `python3 -m unittest discover -s scripts -p 'test_*.py'`.
- [x] 2. **Plain review markers.** `scripts/review.py` opens its markers with mode `0o644` (decision 2). Verify: a test
  in
  `scripts/test_review.py` that a marker is not executable.
- [x] 3. **Tests and docs handoff.** The cases of the design's Tests section; `docs-handoff.md` gives
  `docs/how-to/releasing.md` the new row and the changed sentence; nothing blocks. Verify: the script tests.
- [x] 4. **Finishing checks.** `python3 -m unittest discover -s scripts -p 'test_*.py'`,
  `openspec validate gate-openspec-reuse --strict`, commit, then `python3 scripts/gate.py run` (a full run: the
  policy changed; wait for the lock in short tries, then run it as its own background command with a 2-hour timeout).
