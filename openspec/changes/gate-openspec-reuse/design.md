# Design

## Decisions

1. **A new allowlist key, `any_under`.** `allowed_path` accepts a path under a folder of `any_under` whatever its
   extension; `markdown_under` keeps its Markdown rule for the other folders; `except_under` still wins. The policy
   lists `"any_under": ["openspec/"]`. Not taken: widening `markdown_under` (it would let non-Markdown into `docs/` and
   the skills, which tests and doctests read).
2. **Executable files count only there.** `classify` accepts mode `100755` as well as `100644` for a path under
   `any_under` (review markers are written executable); symlinks (`120000`), submodules and mode changes outside it stay
   refused. Not taken: changing `review.py` to write `0644` markers (old markers in the archive stay executable).
3. **Why it is safe.** No build step, test or doctest reads `openspec/`: Cargo never looks there, the Markdown link
   check
   skips `openspec/changes/archive`, and the scripts' tests use their own fixtures. The docs record still runs the docs
   checks over the tree, so a broken link in an open change's Markdown is still caught.

## Tests

`scripts/test_gate_evidence.py`: an archive-like change (a moved `.openspec.yaml`, an executable `.taken`, Markdown
under
`openspec/`) is allowed; a symlink under `openspec/`, an executable file under `docs/`, and a non-Markdown file under
`docs/` stay refused; `python3 -m unittest discover -s scripts -p 'test_*.py'` passes.

## Docs

The handoff gives `docs/how-to/releasing.md` the new allowlist row (any regular file under `openspec/`).
