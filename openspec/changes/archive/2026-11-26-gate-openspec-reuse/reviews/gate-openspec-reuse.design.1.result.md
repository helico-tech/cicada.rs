Verdict: not yet

## Blocking

1. **Contradicts ADR 0046.** Its Decision says "A narrow allowlist of regular Markdown files may use fresh docs checks". This change lets in any regular file, executable too, under `openspec/`. Accepted ADRs are not rewritten (docs/adr/README.md), so the change needs a short ADR that partly supersedes 0046 for this point, plus its row in the ADR index and in 0046's row ("Partly superseded by NNNN"). Name it in design.md and in Impact.

## Safety (the focus): yes, it is safe

Nothing a full run checks reads non-Markdown under `openspec/`. Cargo has an explicit `members` list with no globs. No Rust test walks `openspec/` except `examples/tests/docs.rs`. That test only reads `.md` files, skips `openspec/changes/archive`, and runs in the docs kind. `skill.rs` only uses `openspec/` as a route-prefix string. `test_status.py` and `test_review.py` build temporary repos, and `python-tests` runs in the docs kind anyway. fmt, clippy, feature-builds, client-tests and deny never look there. The sample-service Docker context does include `openspec/`, but only E2E uses it, and E2E is reused from the same base.

## Notes

1. Impact leaves out `parse_policy` (gate.py:211). It requires exactly `files`, `markdown_under` and `except_under`, so the new key fails there. The parsing test in `test_gate_policy.py` also needs the new key.
2. A simpler way: `review.py:118` calls `os.open(..., O_CREAT|O_EXCL|O_WRONLY)` without a mode, so markers come out 0755. Passing `0o644` fixes the cause. Old markers are already committed and never show up in a new diff (`.reviews/` is ignored). Then `any_under` can keep the rule that files must not be executable.
3. "Mode changes outside it stay refused" has two readings: is a 100644→100755 change *inside* `openspec/` allowed? Say which, and test it.
4. Once `any_under` holds `openspec/`, the `openspec/` entry in `markdown_under` does nothing. Remove it.
5. The handoff should also fix the table row in releasing.md, "Only regular non-executable Markdown changes qualify" (line 148).

## Questions for the owner

None.

## Not checked

Whether the GitHub manual CI workflow reads the policy keys. I did not run the tests.
