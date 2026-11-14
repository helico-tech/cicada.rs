# Design

## One handoff per change, with the current ownership boundary

ADR 0045 adds documentation coordination to the three loops. Use the existing protected() boundary and exceptions;
change no hook, ownership cursor or scheduling. Builders own code/rustdoc/examples, consumer skill routes/version,
ADRs, generated references, changelog and change artifacts. Docs owns README prose and other handwritten docs,
including architecture, guarantees, install/upgrading and operations. The release loop owns README tag substitutions.
No temporary builder edits to protected prose, even as a way to test a proposed replacement.

Every built change includes docs-handoff.md, even with no user-facing effect (say None and why). Each release passes
its version for docs-owned install examples. Keep the template in builder.md only: version; source behavior/public
items and proving tests/examples; destination files and heading text with proposed substance; caveats/upgrade limits;
blocking items (or none), each with failed check, file/block and proposed replacement; actual verification and docs
status. Use code spans for destination paths/headings, not fragile links to future anchors. A guide example points
at compiled example code. Distinguish proposed content, owner-prepared/integrated fixes, and prose not yet written.

Update the handoff with every behavior-changing fix/rebase. It does not name its own commit SHA: the review request's
head identifies reviewed source, and archived reviews preserve that evidence. A blocking request sent to docs names
its concrete feature head; owner-prepared artifacts may be recorded by their separate commit and affected paths.
After landing, the archived handoff plus landed source is the docs session's input, not an unimplemented proposal.

## Additive changes and existing tasks

When code, doctests, links, guarantees and skill routes pass without prose edits, Blocking is none. The builder runs
normal checks; code review checks the handoff against its exact head. Present distinguishes handed-off guidance from
written guidance. Landing reports the archived handoff for the docs scan. Asynchronous prose upkeep has no guaranteed
latency and is not a reason to claim missing prose is already updated.

Existing ready tasks that request protected edits translate into handoff content under AGENTS.md; do not rewrite
their folders. They do not translate away verification obligations. New designs identify affected doc blocks, links,
guarantee rows and skill routes; tasks request handoff and, if needed, a coordinated blocking fix. Compatibility
aliases can avoid disruption when deliberately designed, but are not silently added by a builder.

## Blocking checks: prepare with the owner, integrate exactly, then verify

A breaking example, removed test named by guarantees, broken link or route is blocking. The builder finishes other
work and writes exact evidence/replacement suggestions in the handoff, leaves those verification tasks unticked and
reports blocked on docs. No formal ready-for-code-review report or pass while required checks fail; an accurately
reported failure remains a blocker. The build loop records the source head/paths and requests owner preparation.
If the docs owner is unavailable, keep the work and report the concrete blocker; no broken main, disabled test,
feature switch, hook bypass or silent permission expansion. Await owner action, not a guessed approval after time.

The docs owner prepares and validates against the named feature head using its own workflow, without merging feature
code into the docs branch or publishing code-dependent prose on main first. This producer records the needed docs-side
procedure in its own handoff; until adopted, coordination is explicit. It does not change docs-owned maintenance prose.
The docs owner must provide complete affected files compatible with the current main base, preserving unrelated edits.

The builder may integrate those exact prepared files into its feature worktree without merging: restore only the
agreed paths from the named owner commit, including both sides of a rename/deletion. Before committing, verify their
staged blobs/deletions equal the current recorded docs-owner branch, as the existing hook requires. A mere matching
commit subject is not approval: require the owner-provided source head, paths and artifact, inspect the full diff and
check it contains no unrelated feature code. A moved owner tip is harmless only if those path blobs are still equal.

Fold required prose fixes into the corresponding breaking code commits before landing, so the sequence introduced
to main does not contain an intermediate docs break. Do not prescribe broad autosquash that rewrites unrelated work.
Run the blocked checks and normal finishing checks on the combined result; only then tick tasks and request the
normal code review. Review checks both handoff and the integrated owner-prepared diff. Owner permission to land/release
remains the build loop's final gate; this workflow grants no separate push/merge authority to builders or reviewers.

A code fix or rebase invalidating the prepared prose requires refreshed owner preparation. Never resolve a protected
file conflict by blindly choosing one side; the docs owner must reconcile current base, feature behavior and prior
prose. A conflict-free late rebase also requires compatibility checks: a clean Git result says nothing about whether
the prose still describes the behavior. Recheck blob identity and affected checks before review/landing. Any changed reviewed content
uses existing review rules. Final integration preserves code and required docs together; unavailable fixes block it.

## Release pins and bounded scope

The current releasing guide already assigns install-guide dependency pins to docs and README pins to release. Remove
the proposal's stale conflict claim; do not add an install.md hook exemption. Use docs/how-to/releasing.md in the loop,
set skill metadata with the workspace version, and include version/pin refresh in every handoff. Builders maintain
skill routes for new examples and coordinate guide/heading changes with docs; route checks remain mandatory.

Update builder/build/design/review briefs, config task rules and AGENTS.md pointers consistently. The builder's report
has an explicit Docs state: handed off with no blockers, blocked, or required owner fixes integrated (remaining prose
still pending). A reviewer never turns a known failing check into a pass because it is listed in Blocking.

Keep the owner's current Order after flow-finalizer; no priority change to a running build. This skills/config change
uses the normal implementation patch release and a process-only changelog line; it is not the docs-only maintenance
exemption. ADR 0045 records the coordination/gate, not a library guarantee or security boundary. No runtime changes.

Verification is existing hook tests plus docs links/doctests/skill checks and a review walkthrough of additive,
breaking, absent-owner, changed-head (including a conflict-free late rebase) and release-pin cases. The code-review
brief explicitly accepts this evidence for process scenarios in its scenario-test rule. No mirror-text tests or new workflow engine. Four tasks
plus finishing. The separate docs session owns adoption of its side; the handoff must not claim automatic discovery
of unpublished feature heads already exists.
