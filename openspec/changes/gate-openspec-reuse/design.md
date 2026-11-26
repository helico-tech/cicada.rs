# Design

## Decisions

1. **A new allowlist key, `any_under`.** `allowed_path` accepts a path under a folder of `any_under` whatever its
   extension; `markdown_under` keeps its Markdown rule for the other folders and loses its now-redundant `openspec/`
   entry; `except_under` still wins. `parse_policy` (which accepts exactly the known keys) and its test learn the key.
   Not taken: widening `markdown_under` (it would let non-Markdown into `docs/` and the skills, which tests read).
2. **Review markers become plain files; the non-executable rule stays.** The markers are executable only because
   `scripts/review.py` opens them without a mode; it passes `0o644`. Markers already committed never reappear in a new
   diff (`.reviews/` is ignored by git until a landing moves the files), so `classify` keeps accepting only mode
   `100644`, and a mode change inside `openspec/` stays refused, like symlinks (design review, note 2 and 3). Not taken:
   accepting executable files under `openspec/` (it would weaken the rule for one cause that can be fixed).
3. **Why it is safe** (design review): Cargo lists its members by name and never reads `openspec/`; the only Rust test
   that walks it, `examples/tests/docs.rs`, reads Markdown only and runs in the docs checks; the script tests build
   their
   own repositories; the sample-service image copies `openspec/`, but only E2E uses it, and E2E is reused from the same
   base. The docs record still runs the docs checks over the tree.
4. **An ADR.** ADR 0046 allows only regular Markdown files; ADR 0070 partly supersedes it for `openspec/`.

## Tests

`scripts/test_gate_evidence.py`: an archive-like change (a moved `.openspec.yaml`, a plain review marker, Markdown under
`openspec/`) is allowed; a symlink under `openspec/`, an executable file there, a mode change there, and a non-Markdown
file under `docs/` stay refused. `scripts/test_gate_policy.py`: the `any_under` key is accepted.
`scripts/test_review.py`:
markers are written `0o644`. `python3 -m unittest discover -s scripts -p 'test_*.py'` passes.

## Docs

The handoff gives `docs/how-to/releasing.md` the new row (any non-executable regular file under `openspec/`) and changes
its sentence "Only regular non-executable Markdown changes qualify".
