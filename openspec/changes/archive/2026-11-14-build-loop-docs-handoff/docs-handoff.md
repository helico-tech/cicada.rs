# Docs handoff for build-loop-docs-handoff

Version: 0.18.6. Refresh the dependency pins of `docs/how-to/install.md` once it is published. The library is that of
0.18.5: no API, wire format, schema, guarantee, CLI or HTTP route changes, so the guides, `docs/guarantees.md`,
`docs/architecture.md`, `docs/operations.md`, `docs/how-to/upgrading.md` and the README need nothing for users.

## What changed

Only the loops that build Cicada (ADR 0045). The code branch holds:

- `.claude/skills/build-loop/builder.md`, section `## Documentation`: the ownership boundary (README.md, AGENTS.md and
  `docs/` except ADRs and generated references), its precedence over tasks, the `docs-handoff.md` every change writes
  (with its template), the blocked report, the integration of owner-prepared files, and item 7 of `## Report`, the
  docs state. Its version step now also sets the skill's `cicada-version` and regenerates `docs/openapi.json`.
- `.claude/skills/build-loop/SKILL.md`, new section `## Blocked on docs`, a Docs line in `## Present`, the recheck in
  `## Land` step 1, `docs/how-to/releasing.md` in step 5, and the archived handoff in step 7.
- `.claude/skills/design-loop/SKILL.md` (Describe, steps 3 and 6), `.claude/skills/review-loop/design.md` (Docs),
  `.claude/skills/review-loop/code.md` (Check 2, 5, 6 and What blocks) and `openspec/config.yaml` (design and tasks).
- ADR 0045, with a closing paragraph that names these places, and its index row.
- The changelog entry, version 0.18.6 in `Cargo.toml`, `Cargo.lock`, `docs/openapi.json` and the skill's metadata.

## 1. AGENTS.md, the contributor rules

**Where.** `AGENTS.md`, `## Rules`, `## Map` and `## Specs and changes: OpenSpec`.

**Text.** Proposed substance; the docs session words it:

- `## Rules`, the bullet **Documentation ownership**: add "Builders write what users must learn into
  `openspec/changes/<change>/docs-handoff.md` (template in `.claude/skills/build-loop/builder.md`). A doctest, link,
  guarantee row or skill route that cannot pass without new prose keeps the change from landing until the docs session
  prepares the fix (ADR 0045)."
- `## Rules`, the bullet on the Rust blocks of README, the guides and `docs/operations.md`: after "a change to a public
  API must keep them compiling", add "through a fix the docs session prepares (ADR 0045)". Replace "User-facing
  behaviour is documented in the guide of its layer" with "User-facing behaviour is documented in the guide of its
  layer, by the docs session from the change's handoff".
- `## Rules`, the bullet on `docs/guarantees.md`: "a change that adds a guarantee, or a test that proves one, adds it
  to the table" becomes "... has the docs session add it to the table (its handoff names the row)".
- `## Rules`, the bullet on `skills/cicada/SKILL.md`: "Missing consumer advice goes into its guide" becomes "Missing
  consumer advice goes into its guide, through the docs handoff".
- `## Rules`, the bullet on releases: point at `docs/how-to/releasing.md` rather than `docs/operations.md`, Releasing
  (that section only links there).
- `## Map`, row `.claude/skills/`: "`build-loop` with the builder's brief and its docs handoff template".
- `## Specs and changes: OpenSpec`, the Flow bullet: after "They hand over through files", add "; a built change hands
  user prose to the docs session in `docs-handoff.md` (ADR 0045)".

**Caveats.** The hook does not cover `AGENTS.md`; it is docs-owned by the owner's rule, so builders hand it off too.

## 2. The docs session's side, in its maintenance procedure

**Where.** `docs/maintaining/maintenance.md`, `## Every maintenance run`, or a new section after it.

**Text.** Proposed substance. The build loop now asks explicitly, with a feature head, the handoff and the paths of
its Blocking items, when a change cannot land without new prose. The docs session:

1. Prepares exactly those paths on the docs branch, as whole files against current `main` that keep its other edits,
   and checks them against the feature head without merging feature code into the docs branch (for example in a
   detached scratch worktree of that head, with the prepared files copied in, running the doctests, the docs test and
   the skill test).
2. Commits only those paths and answers with the commit and the paths, both sides of a rename or deletion included.
3. Does not merge that commit into `main` before the feature lands: the prose describes code `main` does not have yet,
   so its doctests would fail there. After the feature lands with the same files, the paths agree and the normal run
   goes on.
4. Prepares again when the build loop reports that the feature head or base changed what the prose describes, or that
   the docs branch changed those paths since.

Also: after each landing, read the archived `docs-handoff.md` that the build loop names
(`openspec/changes/archive/<date>-<change>/docs-handoff.md`); it lists the prose and the version to publish.

**Caveats.** There is no scan for unlanded feature branches; until the docs session adopts this procedure, the owner
relays each request by hand and the build stays blocked meanwhile. The hook compares integrated files with the docs
branch's current tip, so a later docs commit to the same paths before the feature lands makes the builder ask again.

## Blocking

None. Every doctest, link, guarantee row and skill route passes without prose edits.

## Status

Checks run on the code branch: `cargo test -p cicada-examples --test docs` (11 passed), `openspec validate
build-loop-docs-handoff --strict` (valid), `cargo test -p cicada-examples --test skill` (8 passed), `python3 -m unittest
scripts/test_docs_maintenance.py` (8 passed, the hooks unchanged), `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets` (clean) and `cargo test --workspace` with PostgreSQL (2905 passed, 0 failed). Walkthrough, clean late
rebase: the paths still equal the docs branch and no behaviour changed, so no fresh preparation, but the builder still
runs the doc, docs and skill tests on the rebased branch, and the loop merges only when they pass (build loop, `##
Blocked on docs` step 3 and `## Land` step 1). Items 1 and 2 are proposed text; nothing above is written yet in the
docs-owned files, and no owner fix is integrated.
