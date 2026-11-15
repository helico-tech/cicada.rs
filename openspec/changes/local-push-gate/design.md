# Design

## One checked path from commit to publication

ADR 0046 replaces automatic Actions with local evidence and publication. Keep one change: splitting before the release
transition would leave an intermediate version requiring the billing-blocked workflow. Five implementation tasks are
policy/commands, execution/evidence, hook, release driver, and workflow/brief/handoff integration. No library changes.
Keep manual Linux CI, remove automatic push/PR/tag triggers and release.yml. No repairs or claims about past releases.

`scripts/gate.py`: `checks --scope full|docs|e2e` runs commands without evidence in the current directory (manual CI);
`run [REV] [--e2e] [--force]` validates a pinned commit in an owned worktree; `status [REV]` explains evidence;
`install-hook`, `pre-push REMOTE URL`, and `release TAG [REV]` complete the interface. Release requires normal owner
approval through the build loop; builders only test local temporary remotes/fake GitHub, never install or publish.

## Policy and full coverage

Read versioned `scripts/gate-policy.json` from commit C using Git objects: supported schema, argv command lists and
explicit docs allowlist. Shared scripts under scripts/ci hold current multi-line feature/dependency checks. The
runner implementation blob must match C's gate.py; refuse unsupported/missing policy rather than applying new rules
to old code. Record that blob and policy blob. Hook validators reject unknown schemas and inconsistent evidence.

Full = Python test discovery (`scripts/test_*.py`), fmt check, clippy workspace/all-targets with -D warnings, workspace
tests with PostgreSQL, core no/sqlite/postgres/testing and app no/sqlite/postgres/tui checks with -D warnings, app-alone
tests, and every current tui/driver/TLS/crypto-provider dependency-tree check. Docs = Python tests plus examples doc,
docs and skill tests. E2E = the complete Docker suite with CICADA_E2E=1 and no filter. Preserve every former CI check.

Use a clean command environment: retain PATH, HOME, USER, LOGNAME, SHELL, TMPDIR, locale/TERM, CARGO_HOME/RUSTUP_HOME,
RUSTUP_TOOLCHAIN, Docker connection/config variables and SSH_AUTH_SOCK. Set line-tables-only, Python no-bytecode and
the configured test PG URL; feature checks set their own RUSTFLAGS/target/deny. Strip all other caller overrides,
notably CICADA_UPDATE_*, CICADA_E2E_ONLY, PROPTEST_*, RUST*_FLAGS/wrappers, CARGO_TARGET_DIR and test filters. Git/GitHub
publication uses its ordinary auth environment separately; never put credential values in evidence or argv logs.
Read CICADA_GATE_PG_URL (default repository compose URL); require a nonempty URL and real test execution. A TCP probe
is only a diagnostic, not proof of database/auth access. A bad URL must fail, not skip. Keep cargo's normal caches;
refuse external ancestor/CARGO_HOME Cargo config and ancestor rust-toolchain files, rather than silently inheriting
untracked compiler policy. This is controlled local execution, not a hermetic build or security sandbox.

## Owned checkout and interruption

All mutating gate commands take one common-dir/cicada-gate/lock with flock; busy/unsupported fails visibly. Release
holds it throughout and calls check functions directly, never recursively invoking a locking gate command. Hook and
status only read. Pass the inheritable lock descriptor to child commands; run them in process groups, terminate/reap
on INT/TERM. A killed parent cannot authorize another checkout while a surviving holder retains the lock. Before
commands write an in-progress marker; only a normal, reaped completion clears it. An abandoned marker blocks destructive
reuse pending explicit operator recovery, even if the lock is free; do not infer child exit from the parent's death.
Recovery takes the lock, inspects recorded PID/start time/process groups and processes using the owned checkout,
terminates only positively identified gate groups, and confirms no users remain before clearing the marker. If that
cannot be established, refuse cleanup. Preserve failed/running evidence; recovery never turns it into a pass.
Driver/Docker daemons are outside flock; the existing E2E harness removes its own stale stack, never unrelated stacks.

Create only the absent ignored path .claude/worktrees/gate. Verify its registered worktree, matching common-dir and
gate-owner marker in its Git admin directory before checkout/clean. Never adopt an arbitrary existing directory or
touch release-local. Detach/force C, clean ignored/untracked content except owned top-level target/, and verify tree.
Refuse a tracked or symlink target cache. After checks HEAD must still be C and tracked/untracked/ignored changes
outside target must be absent. Retained target/e2e-logs are real logs; do not claim none exist. Tests cannot silently
rewrite release inputs. Unrelated caller worktree changes do not participate and are never cleaned or reset.

## Evidence and conservative reuse

Atomically replace records/<kind>-<tree>.json under the common directory (temporary file, fsync, rename). Store schema,
kind, commit/tree, gate/policy blobs, exact commands and exit codes, status, timestamps and tool versions; docs records
also name base commit/tree. No credentials or environment dumps; PG metadata has only host/port/database, no query.
Starting a rerun invalidates that record to running; failure writes failed, interruption leaves unusable running.
Never fall back to an older pass after --force fails or aborts. No expiry/toolchain matching: evidence means this tree
passed these commands with the recorded environment, not every compiler/database version. --force refreshes it.

Full pass applies to identical trees across commits. Otherwise search at most 100 first-parent ancestors for a direct
full pass B with the same policy/runner, and classify the cumulative raw NUL-delimited diff, rename detection off.
Only regular non-executable file additions/modifications/deletions qualify: README.md, CHANGELOG.md, AGENTS.md,
docs/**/*.md excluding docs/reference/**, skills/cicada/**/*.md, .claude/skills/**/*.md, openspec/**/*.md. Unknown
paths, symlinks, mode changes, executable files, submodules and other OpenSpec files force full checks. Both ends of
a rename count. Audit input consumers; source grep is evidence for the allowlist, not a proof against future consumers.
Run docs checks on C and retain B in its record. No chains of docs bases. E2E is direct for C, or reused from the same
eligible direct-full ancestor with a valid direct E2E pass. Revalidate records, object existence, command lists,
policy/runner identity, base ancestry and the classifier on every use. A failed exact-kind run blocks that kind's
fallback until rerun succeeds. Notes/skill packaging are always checked anew for a release, never inferred from docs.

## Pre-push

Install only an absent or gate-marked pre-push; respect core.hooksPath and refuse foreign hooks with chaining guidance.
Do not replace docs hooks. Read every Git-provided update, with no network: main deletion/non-fast-forward/unknown old
OID is refused; its proposed tip needs valid checks. New strict vX.Y.Z tags need peeled-commit checks plus E2E and
ancestry of the named remote's previously accepted local main tracking ref (fetch first if unknown).
Never use a proposed main update as ancestry evidence: Git can accept a tag while rejecting main in the same push.
Reject every existing remote version-tag mutation/deletion, even if unpublished. Other refs pass. Resolve all lines
before deciding, including HEAD:main and combined pushes. This is local workflow control; --no-verify, another clone,
forged records or an old hook can bypass it. Only pushed tips are checked, not every intermediate commit.

## Pinned release and recovery

Under the lock resolve C once and persist tag/C in release state; retries must use that C. Never modify caller HEAD,
main or dirty files. On the owned C checkout, release-check gains --before-push (version/skill/dated notes only), while
its existing mode still fetches and verifies origin/main ancestry. Pin its selected commit to the checkout, not a
different pre-existing tag. Save exact notes, obtain checks and E2E, then publish only C:refs/heads/main without force.
If origin main already contains C, skip; if behind, fast-forward; if diverged, stop. On a racing rejection refresh
once, then stop if unresolved. Run full release-check on C after main publication. Main is public at this point.

Create an annotated local tag explicitly at C or accept one already peeling to C; pin its raw object O and refuse
subsequent changes. Check local/remote O, distinguishing an annotated tag from a lightweight tag at the same commit.
Package the skill and verify SOURCE/bytes against C before pushing O:refs/tags/TAG, never force. Validate tag identity
before/after packaging; a concurrent ref change stops publication. Existing remote O must match exactly. Remote tags
are immutable from this point, regardless of Release state. No automatic rollback, rewind or tag deletion on failure.

GitHub operations use the repository identified by origin, never the caller's ambient default repo. Inspect matching
releases including drafts. A published one with the expected remote tag is left untouched and reported already
published, not retroactively verified. A draft must have matching tag/title/notes and only the expected asset with
matching sha256 (download if no digest); otherwise stop. Create a draft if absent, upload only a missing matching
asset, verify contents, then publish; never clobber someone else's draft. Reinspect after uncertain API outcomes.
Persist state/artifact hashes atomically and revalidate external state on resume; stages are hints, not authority.
Main/tag pushes survive later failures, drafts remain resumable, published Releases never change. Cross-clone races
remain bounded by server ref checks and pre-publication reinspection, not a distributed transaction.

## Integration and proof

ADR 0046 supersedes 0016's remote workflow and failed-unpublished-tag deletion rules, preserving versions/pins.
Manual CI uses the same full policy and PostgreSQL service, produces no local record and publishes nothing. Delete
release.yml in this same change. Update build briefs to run evidence on committed reviewed source; archive/release
edits get fresh classified checks. Land installs the hook and uses release only after the owner's existing yes.
The first release uses this new code, so its pushed tree already disables automatic workflows; no temporary bypass.
Handoff AGENTS.md and release/maintenance prose to docs, with no known broken doctest/link/route; describe limitations
and recovery. No other worktrees are removed. Normal patch release, no historical release repairs.

Tests use temporary repos, real local bare pushes, injectable command runners and fake gh. Cover all classifier/ref
cases (including a server that rejects only main in a combined push), command parity, dirty inputs, source-changing commands, interrupted parent/live child, abandoned markers,
forced failures, pinned-C races, tag object mismatch, packaging failure before tag push and draft/published retries.
Finishing runs actual full+E2E evidence with PostgreSQL/Docker plus strict validation. Builders never call real gh
publication or install the shared hook. A private Linux manual run remains optional; local macOS passing is not Linux CI.

Design review found the partial-push race above; the resolution forbids using proposed main for tag ancestry and
requires a real bare-remote regression whose server rejects only main. No owner decision remains open.
