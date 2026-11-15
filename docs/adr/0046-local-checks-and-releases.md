# 0046. Local checks and releases

Date: 2026-11-15

## Status

Accepted for local-push-gate. Supersedes ADR 0016's automatic release-workflow rule and its permission to delete a
failed unpublished remote version tag. The versioning, single workspace version and consumer pinning rules remain.

## Context

Automatic CI duplicates local checks on every push and release, consuming private-repository Actions minutes.
Billing interruption can prevent the documented release path from finishing. Local work needs recorded evidence tied
to the Git content being pushed, including documentation that is compiled or checked, and a resumable publication path.

## Decision

Run the existing CI command coverage and Docker end-to-end checks locally against an owned detached checkout of a
pinned commit. Keep a manual Linux CI workflow using the same policy; disable automatic push/PR/tag workflows and
remove the release workflow. No check category is dropped. Records in Git's common directory identify tree, commit,
policy and runner, commands/results, time and toolchain. They are local evidence, not signatures or a security boundary.

Reuse full evidence for an identical tree. A narrow allowlist of regular Markdown files may use fresh docs checks
over a directly full-tested first-parent ancestor, without chaining derived records; unknown inputs force full checks.
Reuse E2E only from the same qualified direct base. Policy or implementation changes invalidate this shortcut.
Starting a forced rerun invalidates its old pass; failure/interruption cannot be reported as success. No automatic
expiry or toolchain equivalence is claimed: the record says which environment passed, and --force refreshes it.

Serialize owned-checkout mutation with a common lock inherited by child commands and an in-progress marker. Refuse
unowned paths or abandoned work; never clean the shared checkout. Checks use controlled environment inputs and refuse
external Cargo/toolchain configuration. Preserve only the owned build cache. This does not promise hermetic builds.

A non-clobbering pre-push hook requires passing evidence for a fast-forward main tip and refuses main deletion.
New version tags require checks and E2E on a commit on main. Existing remote version tags cannot be changed or deleted,
whether or not a GitHub Release exists. Other refs are not gated. Bypassing the hook or forging records remains possible.

The release command pins one commit and tag object, checks versions/dated notes/skill, obtains evidence, pushes that
commit to main without force, verifies ancestry, packages the skill from the exact tag and only then pushes the tag.
It creates and verifies a matching draft through the GitHub API before publishing. Resume rechecks actual refs and
release content; conflicting tags/drafts stop it. Published releases remain untouched. Main and tag pushes are not
rolled back when a later API step fails. No claim is made that publication is atomic across Git and GitHub.

Only the existing owner-approved build loop installs the shared hook or publishes. Builders use temporary repositories
and fake publication tools for tests. Protected AGENTS.md and user documentation change through the docs handoff.
The gate and publication transition land together, avoiding an intermediate release dependent on automatic Actions.

## Consequences

Pushes and releases need local PostgreSQL and, for releases, Docker. Worktrees share evidence and wait on one runner.
The gate retains another build cache and may require explicit recovery after interruption. Tool output can contain
its own diagnostics and E2E logs; gate metadata never stores credentials. Optional manual Linux CI remains paid and
is not automatic coverage for macOS runs. No library API, wire format, store schema or G/S guarantee changes.
