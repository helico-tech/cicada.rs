---
id: gate-openspec-reuse.code.1
kind: code
change: gate-openspec-reuse
round: 1
base: cec3b4fd819606768446bb98fbab39f39ea0537d
head: 4ed0780372e8c150ff25e8998252ba684af2b47d
requested: 2026-11-26T17:47:16+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done gate-openspec-reuse.code.1 <file>`.

## Note from the requester

Version: none (process only)

Builder report (head 4ed07803): tasks 1-4 done. any_under in gate-policy.json, openspec/ out of markdown_under;
parse_policy requires the key; allowed_path order: files, except_under, any_under, markdown_under; classify unchanged
(100644 only); review.py writes markers 0o644 (the new test fails without the fix). Tests: archive-like commit allowed;
plain yaml/text under openspec allowed; executable, mode change, symlink there refused; non-Markdown under skills
refused; a policy without or with a malformed any_under refused. Script tests 105 passed; openspec --strict; gate full
pass of 17f56d34 and a docs pass for the tick commit. No departures. Docs: handed off.
