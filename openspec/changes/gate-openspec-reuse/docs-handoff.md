# Docs handoff for gate-openspec-reuse

Version: none. A process change to the local gate and the review queue; no crate changes and no release, so the
installation pins stay as they are.

## What changed

The docs shortcut of `scripts/gate.py` (ADR 0046, partly superseded by ADR 0070) now accepts any non-executable regular
file under `openspec/`, not only Markdown: the policy's `docs_allowlist` has a new key `any_under: ["openspec/"]`, and
`openspec/` left `markdown_under`. So the commit that archives a change (it moves `.openspec.yaml` and the review
markers into `openspec/changes/archive`) reuses the full pass of the merged change and runs only the docs checks.
Symlinks, mode changes and executable files under `openspec/` still need a full run; outside `openspec/` only Markdown
qualifies, as before. `scripts/review.py` now writes its `.taken` markers as plain files (mode `0644`), so archived
markers no longer arrive executable.

Proved by `scripts/test_gate_evidence.py` (`test_regular_files_under_openspec_qualify`, with an archive-like commit;
the openspec cases of `test_modes_symlinks_renames_and_submodules_force_full_checks`; non-Markdown under the skills
folders in `test_other_inputs_force_full_checks`), `scripts/test_gate_policy.py` (the `any_under` key) and
`scripts/test_review.py` (`test_markers_are_plain_files`).

The first commit after this change lands still needs a full run: the policy file changed, and a base only counts when
its policy is the same.

## 1. The eligible paths of the docs shortcut

**Where.** `docs/how-to/releasing.md`, `Local checks and the pre-push gate`: the sentence before the table of eligible
paths, and the table.
**Text.** Replace "Only regular non-executable Markdown changes qualify:" with "Only regular non-executable files
qualify: Markdown in the folders below, and any file under openspec/:". In the table, drop `openspec/` from the
Markdown row and add a row:

| Eligible paths | Full checks required |
| --- | --- |
| README.md, CHANGELOG.md, AGENTS.md | Crate READMEs and unknown paths |
| Markdown under docs/, skills/cicada/, .claude/skills/ | docs/reference/, non-Markdown files, symlinks and mode changes |
| Any file under openspec/ | Symlinks, executable files and mode changes |

**Caveats.** A future test or build step that reads non-Markdown files under `openspec/` must move them out of this
rule (ADR 0070).

## Blocking

None.

## Status

Ran `python3 -m unittest discover -s scripts -p 'test_*.py'` and `openspec validate gate-openspec-reuse --strict`; the
gate run is reported by the builder. The text above is proposed; no protected prose is written yet.
