# Design

## Scope and authority

The owner delegated design decisions and approval of review fixes/tasks to this session on 2026-11-16, with Opus 5.5
as reviewer. Preserve the earlier choices: one step at a time, concrete stories, user vocabulary, written results
and Mermaid, and both archive/plugin distribution. ADR 0051 extends ADRs 0041/0049/0050. One behavior: a portable
interactive actor-design skill, including the packaging it needs. Five tasks plus finishing checks are sufficient.

1. Add `skills/actor-design/SKILL.md` and `assets/design-template.md`. Frontmatter has only name, description and
   metadata.cicada-version, as the existing skill; name is actor-design, version equals the workspace. Trigger for
   helping a consumer design actor boundaries/protocols, not general implementation or unsolicited review. Existing
   cicada stays the build/test/operate skill and mentions actor-design by name, not a sibling path. Review mode waits
   for actor-design-review; do not claim it exists. No new executable helper, API manual or contributor loop.
2. Use the sibling `cicada/scripts/cicada_source.py`, resolved from the actual installed folder and shell-quoted,
   while cwd stays the consumer project. Both distributions keep the siblings together; no copied resolver to drift.
   Read sibling discovery instructions and keep all status handling (mixed, none, self, no-docs, unavailable) and
   locked/offline target/features behavior. The helper reports cicada's version/provenance, not actor-design's;
   compare installed metadata and, when present, SOURCE.json and disclose differences rather than implying a match.
   A missing/mismatched sibling is reported; use existing manual lockfile/verified-source discovery where possible,
   never auto-install or choose latest main. Generic problem exploration can proceed with source limits recorded.
3. An explicit `Source routes` section names the resolved design guide's nine headings from actor-design-guide and
   existing core/actors/http/operations sections only when needed. The guide/code of the resolved source wins. An
   older source without design.md falls back to its verified core design section/examples and records the limitation;
   do not fetch a newer guide or treat proposals as shipped. This skill ships the method, not a copy of the guide.

## Interaction and saved result

4. Default to a new `docs/design/actors.md` in the consumer root, unless the user names a path or repository rules
   establish another. Inspect existing documents before writing. If the destination belongs to another design, ask
   for a new name; never replace unrelated work. No repository or writing prohibited: keep the same artifact in chat
   and say it was not saved. The bundled template is a starting shape, not a mandatory rewrite of existing documents.
5. Document sections: Source and scope; Progress; the nine steps with the exact names/order of the landed guide
   under From problem to design; Decisions; Open questions. Template and Progress use those names, checked against
   the guide by skill.rs. Progress records current step, each step's draft/confirmed status and the
   next question. Source records paths/revisions/features and uncertainty, without copying credentials. Never claim
   a result confirmed without the user's response or explicit decision delegation. Save proposed results as drafts.
6. Ask at most three focused questions at a time, grounded in a user's story. Draft only the current step, show its
   written result and updated drawing, then wait for confirmation before advancing by default. Step 1 asks whether
   actors are appropriate; stopping with a simpler non-actor design is valid. Hypotheses stay labelled, requirements
   are not invented. If the consumer explicitly delegates decisions, honor that scope and record decisions as
   delegated, with reasons and open uncertainties; silence is never confirmation. Our own autonomy is not a default
   imposed on consumers of the skill.
7. Diagrams evolve with evidence: early domain/context diagram with unknowns, then actor/message/watch map once
   boundaries emerge, lifecycle diagrams for stateful lifecycles, and success/failure sequences per chosen story by
   step 7. Name messages in the user's vocabulary. Show timeout, repeat, reorder, cancel and dependency failure or
   explain inapplicability. Distinguish host calls and atomic commits. Do not invent actors solely to fill a diagram.
8. Resume by reading the saved document and rechecking source selection, not restarting the interview. Revisions
   to an earlier confirmed step mark affected later sections as needing review while preserving their content and
   unaffected decisions. Update only relevant sections. End with a design, its source basis, unresolved assumptions
   and next actions; do not implement, commit or deploy unless separately requested. No auto-commit of the artifact.

## Archive and release compatibility

9. Keep asset name `cicada-skill-vX.Y.Z.tar.gz` and all existing cicada/ members. Add actor-design/, its SKILL.md,
   assets/ with design-template.md and its own SOURCE.json (identical repository/tag/commit/version to cicada's).
   Files are explicit, regular Git blobs from the immutable tag, not globbed from cwd; stable sorted members, modes,
   timestamps and gzip framing remain. Two named roots are allowed; no traversal, duplicate/unexpected members or
   symlink entries. Shared archive validation accepts this root set while preserving the fixtures' single root.
10. Add `scripts/skill-archive-format` containing `2` and a newline as a tag-owned layout marker, not an archive member.
    Missing marker means legacy layout only when the tag has no actor-design tree; a present marker must equal 2.
    Format 2 requires both complete skills and matching versions; a missing second skill never downgrades it.
    This avoids semver cutoffs (historical test fixtures use arbitrary versions) and preserves old archives byte for
    byte. Current release preflight requires marker 2 and both skill versions, plus the existing plugin version.
    No change to old tags, uploaded assets or saved release digests. Copying only old cicada/ remains usable, but
    actor-design requires both siblings; installation/update guidance compares and installs both together.
11. The release driver selects the member set from its pinned commit's marker, never current cwd. Check every
    SOURCE.json against that commit/tag/version/repository and every source file against its tagged blob; none is
    mistaken for a folder. Preserve legacy recovery and fixture archives. Test new and legacy packaging, missing/
    mismatched skills/marker/provenance, unsafe members and release-resume digest checks in existing sandbox tests,
    including format-2 digests saved before a driver update. Resume must verify, never silently replace them.
    The root plugin needs no manifest/discovery change; both skill versions and the plugin track each release.

## Checks, docs ownership and review

12. Extend examples/tests/skill.rs to check both names/versions and the new skill's explicit Source routes section;
    output paths/template paths are not resolved-source routes. Keep full existing cicada coverage and add required
    design routes, template/guide step agreement and no contributor routes. Add
    examples/tests/skill/actor-design-evals.json with staged
    user turns for new design, not-an-actor problem, timeout story, step confirmation/delegation, resume/changed
    requirement, existing-file protection, old/mixed/missing source/sibling and both installation layouts. CI checks
    shape/coverage, not model compliance or live plugin discovery. Manually check /cicada:actor-design discovery
    when a suitable harness is available and record limits. Behavioral review exercises representative transcripts, checks diagrams,
    no premature advancement, evidence/assumptions and preserved text; no paid model service enters the gate.
13. The docs session owns install/update guidance for both folders in docs/how-to/install.md, entry points in
    docs/README.md and docs/agents.md, and both version pins/format marker in docs/how-to/releasing.md. Keep existing
    anchors. Handoff names the default consumer artifact path, source fallback, plugin names and the scope of each
    skill. The prerequisite guide must already exist at build time; unexpected protected-prose route/doctest failures
    block landing for an exact docs-owner fix. No guide text, G/S row, Rust doc block or generated API needs changing.
    Builder owns skill/template, routes/evals, packager/release tests and ADR/index. Verify strict OpenSpec, targeted
    packaging/skill/release tests, docs/doctests and full committed-tree gate including PostgreSQL/Docker. Record
    actual behavioral/vendor validation and limits; preserve legacy reproducibility evidence.
