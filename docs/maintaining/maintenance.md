# Maintain documentation from the docs session

The owner requested one docs session, a persistent `docs` worktree, automatic updates, and merging into main when
ready. The owner explicitly exempted documentation-only updates from library releases on 2026-11-14.
This is a documentation workflow alongside the normal design/build/review loops.

## Ownership

Run `python3 scripts/docs-maintenance.py claim` from the docs worktree to install local commit/merge hooks and a shared
owner record in Git’s common directory. `status` reports the owner and last reviewed main commit.

The docs session owns README, AGENTS.md and handwritten documentation. Other agents report source/test changes and
leave prose to this session. ADRs, generated CLI/OpenAPI/contract references, and README release-pin substitutions remain
with their producing change; this session reviews their effect on user guidance. This preserves contributor and
release workflows while preventing competing prose edits.

The hooks protect README and handwritten `docs/` files, excluding ADRs and generated references; AGENTS.md is
docs-owned by instruction, outside the hook. They allow exact content prepared on the docs branch to be integrated
and leave unrelated hooks untouched. These are **local Git workflow controls**: they cannot prevent filesystem edits,
hooks being bypassed, or remote pushes. All agents run as the same OS user; real access isolation would require separate credentials/accounts and server-side protection. Do not claim session-level security.

Only this session may advance its cursor or release ownership. `python3 scripts/docs-maintenance.py release` removes
this session’s hooks/owner record when the owner retires it. Pause the recurring task separately.

## Every maintenance run

1. Confirm this is the recorded docs worktree/branch. Check both its status and main’s status. Preserve others’ work.
2. Fetch `origin/main` when reachable; inspect local main as well. A failed fetch is a limitation, not evidence of no changes.
   Check the latest published release too: main can hold a new version before its tag and skill archive exist.
   Keep installation commands on published releases; revisit these pins after publication even if main has no new commits.
3. Run `python3 scripts/docs-maintenance.py scan`. Read the diff, tests, changelog and archived `docs-handoff.md`
   for commits since `reviewed_main`. The build loop reports its path under `openspec/changes/archive/<date>-<change>/`.
   Reconcile the handoff with source; its requested prose is not evidence that the prose exists.
   A non-linear ref needs manual reconciliation; never reset or force-update a branch.
4. Merge the reviewed main commits into docs before editing. Cover user-visible behavior in the relevant layer, link
   complete examples, and preserve architecture guarantees, accepted ADRs and generated references.
5. For feature-dependent fixes awaiting landing, follow the blocked-build procedure below; never merge those into
   main through this routine. Check other prose against [the writing standard](writing.md). Run docs links/guarantees,
   all documentation doctests, and any targeted example/maintenance tests affected. Review the diff; stage only owned files and commit.
6. Recheck main immediately before integration. Merge after checks pass and the docs worktree is clean.
   Preserve unrelated work in main: inspect its diff and integrate only when changed paths do not overlap;
   otherwise wait for that work to finish.
   The user authorized these documentation merges. Stop on conflict or unresolved behavior; never overwrite a builder’s work.
   Documentation-only upkeep needs no library version bump, release changelog entry, OpenAPI version change or tag.
   Runtime changes belong to their builder and retain the normal [release procedure](../how-to/releasing.md).
7. Fast-forward docs to the integrated main, then `acknowledge <exact-reviewed-main-sha>`. Advance only after review,
   verification and integration. If there are no source changes, acknowledge the inspected main without a no-op docs commit.

Keep the checkpoint on failure so the next run sees the unfinished work. Inspect self-authored docs merges without
rewriting them again. Ignore proposals as shipped behavior: an ADR or OpenSpec task alone is not implementation.

## When a build is blocked on docs

[ADR 0045](../adr/0045-docs-handoff-between-loops.md) lets builders integrate this session's exact files when prose
blocks doctests, links, guarantees or skill routes. Handle an explicit request naming the feature head, its
`docs-handoff.md` and Blocking paths. There is no automatic scan of unlanded branches or separate request database.

1. Inspect the concrete head, current main and complete requested files. Prepare only those paths on the docs branch
   as whole files against current main, preserving other edits. Keep feature code out of the docs branch.
2. Verify the files against the feature head, for example in a detached scratch worktree with just the prepared files
   copied in. Run `cargo test -p cicada-examples --doc`, `--test docs` and `--test skill`, plus affected targeted checks.
   Resolve source/prose uncertainty before handing off; a failing check still blocks landing.
3. Commit only the prepared paths and report the commit, feature head, checks and exact paths, including both sides
   of renames and any deletions. The builder integrates these files into the breaking change; it never merges the
   docs branch or independently resolves protected-file conflicts.
4. **Keep this commit off main until the feature lands.** It describes unlanded code and may fail checks against main.
   Track its pending head/paths in this session and exclude it from recurring automatic merges. Other maintenance
   must preserve those files. On landing, verify that main contains the same files before resuming normal maintenance.
5. Reconcile again if the feature/base changes what the prose describes or the docs branch changes those paths.
   The hook compares against the docs branch's current tip. Even unchanged files require the builder's doc, docs and
   skill checks after each fix round or rebase, including a clean rebase; passing old checks is insufficient.

Nonblocking guidance can follow landing through the normal scan. Builders retain source, rustdoc, examples, skill
routes/version, ADRs, generated references and changelog ownership. This procedure grants no additional release
permission and does not change the documentation-only release exemption.

## Scheduling

Use a recurring task bound to this session, every 10 minutes. It runs this procedure in the recorded worktree,
coalescing all commits since the cursor. The owner requested ongoing maintenance; no shell polling daemon is needed.
The scheduler must be enabled and the machine/runtime available; this is periodic review, not a guarantee of an update
at the instant a source file changes. Report conflicts, failed checks or unavailable fetches with the checkpoint intact.
