# Docs handoff for actor-design-skill

Version: 0.19.3. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

A second consumer skill, `actor-design`, ships beside `cicada` (ADR 0051). No Rust API, wire format, contract,
schema or guarantee changes.

- `skills/actor-design/SKILL.md` holds a method; the knowledge stays in the resolved source's
  `docs/guide/design.md`. It works through the guide's nine steps one at a time, asks at most three questions per
  turn from the user's concrete stories, names things in the user's words, shows each step's written result and a
  Mermaid drawing, and moves on only after the user confirms. Step 1 asks whether actors fit; a design without actors
  is a valid end. An explicit delegation by the user is honoured within its scope and recorded under Decisions with
  reasons (status `delegated`, never `confirmed`), also for revisions and the no-actors ending inside that scope.
  It never implements, commits or deploys.
- **Saved design.** Default `docs/design/actors.md` in the consumer's repository; a path the user names or the
  repository's own rules win. A new document starts from `skills/actor-design/assets/design-template.md`: Source and
  scope, Progress (current step, each step's status, next questions), the nine steps, Decisions, Open questions. It
  never overwrites another design (asks for a new name), shows the document in chat when it cannot or may not write,
  resumes from Progress, and marks later steps `needs review` when an earlier result changes.
- **Source.** It runs the sibling `cicada` skill's resolver (`<skills folder>/cicada/scripts/cicada_source.py`, actual
  path quoted, from the project folder) and keeps its statuses. The report's `skill_version`/`skill_source` describe
  `cicada`; `actor-design` compares them with its own metadata and `SOURCE.json` and discloses differences. A missing
  sibling or source is reported and recorded; no install, fetch or fallback to `main`. An older source without
  `docs/guide/design.md` falls back to its `docs/guide/core.md#design-an-actor`, examples and code.
- **Scope of each skill.** `cicada`: build, test, operate, upgrade (its description and intro now name
  `actor-design` for design work). `actor-design`: design with the user before code. Review of an existing system is a
  later change (`actor-design-review`).
- **Distribution.** The plugin `cicada@cicada` carries both skills with no manifest change; the new one is
  `/cicada:actor-design`. The release archive `cicada-skill-vX.Y.Z.tar.gz` (same name) holds `cicada/` as before plus
  `actor-design/` with `SKILL.md`, `assets/design-template.md` and a `SOURCE.json` identical to `cicada/SOURCE.json`
  (format 2). The tag's `scripts/skill-archive-format` (`2` and a newline) selects format 2; a tag with neither that
  file nor `skills/actor-design` still packages the single-skill layout byte for byte.
- **Release checks.** `scripts/release-check.sh` also requires `scripts/skill-archive-format` to hold format 2 and
  `metadata.cicada-version` of `skills/actor-design/SKILL.md` to equal the release. `gate.py release` takes the archive
  layout from the pinned commit and checks both `SOURCE.json` files (repository, tag, commit, version) and every
  packaged file against its regular blob there.
- Tests: `examples/tests/skill.rs` (`the_frontmatter_names_the_skill_and_the_workspace_version` for both skills,
  `the_design_skill_routes_resolve_and_cover_the_design_guide`, `the_design_template_and_method_follow_the_guides_nine_steps`,
  `the_design_skill_names_no_contributor_material`, `the_two_skills_point_to_each_other_by_name`,
  `a_missing_design_route_step_or_section_is_named`, `the_design_evaluation_cases_are_complete`);
  `examples/tests/skill/actor-design-evals.json` (16 staged cases); `examples/tests/skill/test_package_skill.py`
  (format 2, legacy bytes, marker, versions, links, unsafe members); `scripts/test_gate_release.py` (format-2 release,
  release-check cases, legacy release and resume, driver checks of both `SOURCE.json` files and files, resume against
  saved digests, an updated driver refused and the pinned driver resuming).

## 1. Install both skills

**Where.** `docs/how-to/install.md`, `Install the coding-agent skill`. Keep the heading and its anchor.
**Text.** Say there are two consumer skills: `cicada` finds the project's resolved Cicada source and routes building,
testing and operating; `actor-design` works out an actor design with the user step by step and saves it in
`docs/design/actors.md` (by default). After the plugin install, look for `/cicada:cicada` and `/cicada:actor-design`.
For the archive, from 0.19.3 on it holds two folders; a first install copies both side by side:

```sh
mkdir -p .claude/skills
cp -R "$staging/cicada" "$staging/actor-design" .claude/skills/
```

**Caveats.** `actor-design` needs its sibling `cicada` beside it: it runs that resolver. Installing `cicada` alone
stays valid. Archives before 0.19.3 hold only `cicada/`.

## 2. Update both skills

**Where.** `docs/how-to/install.md`, `Update the installed skill`. Keep the heading and its anchor.
**Text.** For an archive installation, compare and replace both folders, after reviewing local changes:

```sh
diff -r "$staging/cicada" .claude/skills/cicada
diff -r "$staging/actor-design" .claude/skills/actor-design   # absent on a first update from an older archive
rm -rf .claude/skills/cicada .claude/skills/actor-design
cp -R "$staging/cicada" "$staging/actor-design" .claude/skills/
```

Keep both folders at the same version; the switch to the plugin compares both folders with the plugin's.
**Caveats.** The design documents a consumer saved (`docs/design/actors.md`) are the consumer's files; updating a skill
never touches them or the Cicada dependency.

## 3. Entry points

**Where.** `docs/README.md`, `Work with an agent`; `docs/agents.md`, `Read only what the task needs` (row `Design actors
and protocols`).
**Text.** README: next to installing the consumer skill, one line that the `actor-design` skill designs a system with
the user step by step and saves the result as a Markdown document with diagrams. Reading map: the design row can say
that the `actor-design` skill walks through that guide's nine steps with the user.
**Caveats.** Neither skill is a substitute for the resolved source's guides.

## 4. Releasing

**Where.** `docs/how-to/releasing.md`, step 1 and the paragraph `When the next implementation change lands`.
**Text.** Keep `metadata.cicada-version` of both `skills/cicada/SKILL.md` and `skills/actor-design/SKILL.md` and
`version` in `.claude-plugin/plugin.json` equal to the workspace version; the skill test and the release checker
enforce all three. `scripts/skill-archive-format` holds `2` and a newline; the release checker requires it. Step 4: the
skill archive holds `cicada/` and `actor-design/`. When the next change lands, set the version in both skills.
**Caveats.** Do not change the format file without a new ADR and packager support. Old tags, their archives and saved
release digests are never rewritten; a tag without the format file and without `skills/actor-design` packages as before.

## Blocking

None. No doctest, link, guarantee row or skill route needs protected prose.

## Status

- `cargo test -p cicada-examples --test skill`: 16 passed, 0 failed (includes the Python resolver and packaging tests).
- `python3 -m unittest` of `examples/tests/skill/test_package_skill.py`: 16 passed; `scripts/test_package_fixtures.py`:
  10 passed; all script tests (`scripts/test_*.py`): 89 passed, including the format-2 resume after a driver update
  (`test_a_format_two_resume_after_a_driver_update_runs_the_pinned_driver`).
- Sections 1 to 4 are proposed text; nothing protected was edited. The contributor brief
  `.claude/skills/build-loop/builder.md` still names only `skills/cicada/SKILL.md` for the version; the build loop's
  owner may add `skills/actor-design/SKILL.md` there (the skill test fails on a stale version either way).
- Behavioral evidence and limits: `openspec/changes/actor-design-skill/behavioral-review.md`.
