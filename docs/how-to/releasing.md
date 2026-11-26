# Release Cicada

Documentation-only updates from the dedicated docs session merge after checks without a library release,
version bump or tag (owner decision, 2026-11-14). Implementation changes use the procedure below.

A release is a tag `vX.Y.Z` on `main` (ADR 0016). All crates share the version in `[workspace.package]`.

Prerequisites: Python 3.11+, the Rust toolchain with fmt/clippy, local PostgreSQL, Docker for E2E, Node 26 or later
as `node` on PATH, `jq`, and `gh` signed in with release rights. The full gate's `client-tests` check runs the example
client's JavaScript tests with Node's built-in test runner; no npm packages are needed. Missing or older Node fails
the check. With nvm, select Node 26+ in the shell running the gate and pre-push hook; the gate preserves PATH.
`CICADA_GATE_PG_URL` defaults to the compose database on port 55432. Only the
owner-approved change loop installs the shared hook or publishes; builder tests use temporary repositories and fake
publication tools. Automatic push, PR and tag Actions are disabled (ADR 0046).

## Land a change

Landing merges, archives and pushes a change to `main`; it does not release it
([ADR 0055](../adr/0055-one-change-loop-a-fast-lane-and-batched-releases.md)). The change loop commits the archive
at landing, runs `python3 scripts/gate.py run` on the new tip with the full checks, then pushes main normally.
Up to two builders may run at once on changes that share no crate
([ADR 0073](../adr/0073-two-builders-in-the-change-loop.md)). The change loop lands them one at a time, in the order
they pass review: it rebases each on `main`, checks its ADR number, and runs the gate on the landed tip.
Builders run E2E only when their tasks ask for it: changes to `sample-service/`, `e2e/`, the `serve` or `worker`
commands of `cicada-app`, or the HTTP crates (`cicada-axum`, `cicada-client`). Every release requires E2E evidence.

Landed implementation changes collect under one `## X.Y.Z — unreleased` heading in `CHANGELOG.md`:

- The first change after a release sets the next patch version, or the next minor for a breaking change while the
  version is 0.x; [installation](install.md#upgrade) says what counts as breaking. Later changes add entries under
  that heading. A breaking change raises an unreleased patch version to the next minor, renames the heading and
  rewrites the summary for the whole batch.
- When setting the batch version, the builder updates `Cargo.toml`, runs `cargo check` for `Cargo.lock`, regenerates
  `docs/openapi.json`, and keeps `metadata.cicada-version` in both `skills/cicada/SKILL.md` and
  `skills/actor-design/SKILL.md`, and `version` in `.claude-plugin/plugin.json`, equal to the workspace version.
  README tag lines and installation pins wait for the release.
- A process or documentation change that touches no crate gets `Version: none` and leaves versions and the changelog
  alone. Documentation-only maintenance retains the exemption above.

## Release a batch

Run `python3 scripts/status.py`. Its `release:` line says `release now` when an unreleased batch holds a change
outside the fast lane and no change is ready or in build, or when a landed proposal has `Release: now` in its
`## Lane` section. The owner can also request a release. A batch of fast-lane changes alone waits for a behaviour
change or the owner's request.

1. Fetch origin and confirm `main` equals `origin/main`. The batch's versions are already set. In `CHANGELOG.md`,
   replace `unreleased` in that version's heading with today's date, as `YYYY-MM-DD`, and update the README's
   `tag = "vX.Y.Z"` lines. Commit these two files on `main`; the changes were archived at landing.
2. On the owner's approval in the change loop, run:

   ```sh
   python3 scripts/gate.py release vX.Y.Z
   ```

   The command pins HEAD (or an explicit final `REV` argument), verifies versions and dated notes, obtains local
   checks and Docker E2E evidence, then pushes that commit to `origin/main` without force. It verifies main ancestry,
   creates or accepts a matching tag, packages and verifies the skill and fixtures, and pushes the exact tag object.
   Finally it creates or resumes a matching draft GitHub Release in origin's repository, verifies its notes and both assets,
   and publishes it. A conflicting main, tag or draft stops publication.
3. Confirm the published Release has `cicada-skill-vX.Y.Z.tar.gz` and `cicada-fixtures-vX.Y.Z.tar.gz`.
   The skill archive contains the sibling folders `cicada/` and `actor-design/`, each with an identical `SOURCE.json`;
   the release driver checks provenance and every packaged file against the pinned commit.
   The docs session then updates installation pins from the published release's version, which may differ from earlier
   handoffs in the batch. The fixture archive starts at 0.18.12; older releases are not backfilled.

An E2E failure at release stops further landing. Find the cause in the batch's commits; put a fix first in the line
as its own change with `Release: now`, or revert the change that broke it. Nothing else lands until the release passes.

The skill test and release checker enforce matching versions; the release checker also requires
`scripts/skill-archive-format` to contain `2` followed by a newline.

Code review also records the Claude CLI version, commands and warnings from these checks of the manifests and layout:

```sh
claude plugin validate .claude-plugin/marketplace.json
claude plugin validate .claude-plugin/plugin.json
claude plugin validate .
```

The gate does not require the Claude CLI. Public-directory findings about a missing LICENSE or icon do not block
this private marketplace. Validation does not prove Git access, installation or skill loading
(see [ADR 0049](../adr/0049-consumer-skills-as-a-plugin.md)).

The tag's `scripts/skill-archive-format` selects the archive layout. A tag with neither that file nor
`skills/actor-design` keeps the earlier single-skill layout and bytes. Changing the format requires a new ADR and
packager support; never rewrite old tags, their archives or saved release digests.

## Resume a release

Run the same `release vX.Y.Z` command again. State in `<git common dir>/cicada-gate/release/vX.Y.Z.json` pins its
commit; a retry refuses another commit or raw tag object, including a lightweight/annotated substitution at the same
commit. Its `assets` map pins both filenames and digests; a retry with a different map is refused. It rechecks remote
state and resumes only a draft with matching title, notes and asset hashes. The draft can hold any subset of the two
assets: only missing assets are uploaded, uncertain uploads are re-read, and both must verify before publication.
Duplicate, extra or conflicting assets stop the command without overwrite. A published Release is reported as
already published and left untouched.

If an updated driver produces different archive digests, it refuses the retry. Resume with `scripts/gate.py`
from the pinned commit, whose packager and archive layout reproduce the saved digests; do not replace the digest map.

A state file with the earlier single `asset_sha256` field is refused. Resume that interrupted release using
`scripts/gate.py` from its pinned commit; do not convert its state to the new format.

A failure before the initial tag push leaves no new remote tag. Main and tag pushes survive later failures;
publication is not atomic across Git and GitHub. **Never delete or move a remote version tag**, even without a
published Release or when checks fail. A correction uses a new version. If nothing was pushed and the pinned commit
must change, the owner must inspect actual remote state before manually removing the release state file.

`scripts/release-check.sh [--before-push] vX.Y.Z` checks the checked-out commit and requires an existing local tag to
point there. `--before-push` omits the fetch/main-ancestry check. It is a preflight, not check or E2E evidence.
After tagging, `python3 scripts/package-skill.py vX.Y.Z dist` reproduces the archive from tagged Git objects.
`python3 scripts/package-fixtures.py vX.Y.Z dist` reproduces the fixture archive the same way, with the four fixed
files and `SOURCE.json` identifying the repository, tag, commit and version under `cicada-fixtures/`.

## Local checks and the pre-push gate

Check a committed tree before pushing main:

```sh
python3 scripts/gate.py run             # HEAD, or supply REV
python3 scripts/gate.py status          # inspect evidence for HEAD, or REV
python3 scripts/gate.py run --e2e       # also the unfiltered Docker suite
```

`run` uses the commit's [policy](../../scripts/gate-policy.json) and matching `gate.py`. Full checks cover all
Python script tests, fmt, clippy with warnings denied, workspace tests with PostgreSQL, isolated core/app features
with warnings denied, app-alone tests, driver/TLS/crypto-provider/terminal dependency checks, the example client's
JavaScript tests, and RustSec advisories in `Cargo.lock`.

The full gate's `advisories` check runs `cargo deny check advisories` with the root [deny.toml](../../deny.toml).
Install the version its configuration expects:

```sh
cargo install cargo-deny --locked --version 0.20.2
```

The check fetches the RustSec advisory database each full run, so it needs network access; a missing tool or failed
fetch fails the check and produces no passing evidence. The gate passes no proxy environment variables, so this
check cannot run through an HTTPS proxy. Handle a new advisory as its own change: update the lockfile (`Release: now`
for a vulnerability), or accept the advisory in `deny.toml` with a reason. The current exception is the unmaintained
`chrono`, which has no fixed version and is used by the public API and `sqlx`.

A documentation-only push can reuse an eligible full pass. A batch release can reuse the full pass from landing:
an advisory published afterward is caught by the next full run, not necessarily by the release.

For an identical tree, passing evidence can be reused. Otherwise the nearest eligible direct full pass within
100 first-parent ancestors can support fresh docs checks: Python tests plus examples doctests, links/guarantees and
skill tests. Only regular non-executable files qualify: README.md, CHANGELOG.md and AGENTS.md, Markdown in the
folders below, and any file under openspec/ (ADR 0070):

| Eligible paths | Full checks required |
| --- | --- |
| README.md, CHANGELOG.md, AGENTS.md | Crate READMEs and unknown paths |
| Markdown under docs/, skills/cicada/, .claude/skills/ | docs/reference/, non-Markdown files, symlinks and mode changes |
| Any file under openspec/ | Symlinks, executable files and mode changes |

So the commit that archives a change, which moves its `.openspec.yaml` and review markers, reuses the merged change's
full pass. A test or build step that reads non-Markdown files under openspec/ must move them out of this rule.

The cumulative diff is checked, including both sides of renames. A changed policy/runner or missing eligible full
base needs full checks; derived docs records never form a chain. E2E can reuse that same eligible full base's direct
E2E pass. `--force` reruns checks and any requested E2E; failure or interruption makes the previous record unusable.

Records under `<git common dir>/cicada-gate/records/<kind>-<tree>.json` name commit/tree, policy/runner, commands and
results, timestamps, tool versions and PostgreSQL host/port/database without credentials. They do not expire:
evidence describes the recorded environment, not every toolchain. Use `--force` to refresh it.

### Hook and environment

The change loop installs `python3 scripts/gate.py install-hook` once with the owner's approval. It respects
`core.hooksPath`, preserves docs hooks and refuses a foreign pre-push hook with chaining instructions. The hook
requires checks for the pushed main tip and refuses deletion/non-fast-forward updates. New version tags need checks,
E2E and ancestry of the named remote's previously accepted main: fetch first and push main before the tag. Existing
remote version tags cannot change or disappear. Other refs pass.

Checks use the owned, ignored `.claude/worktrees/gate`, detached at the commit and cleaned except its real `target/`
cache. An existing unowned path is refused; caller worktrees stay untouched. The gate preserves normal path/home,
user/shell/temp/locale, Cargo/rustup/toolchain, Docker and SSH-agent settings, while stripping update flags, filters,
compiler flags/wrappers and target-directory overrides. External ancestor or Cargo-home Cargo configuration and
ancestor toolchain files are refused. A check that changes its source checkout fails.

### Interrupted or busy runs

All sessions share one lock and owned checkout. A busy gate refuses another run; wait for its owner to finish.
Builders and the change loop retry `busy` in short tries for at most 50 minutes; each try runs as a background command
with a 2-hour limit, since the try that gets the lock is the run. Never stop a run once it started.
After interruption, inspect `status`, then run:

```sh
python3 scripts/gate.py recover
```

Recovery takes the lock, verifies that the recorded gate process ended, stops an identified remaining command group,
and checks for processes working in the checkout (`lsof` required). Uncertain process identity or remaining users
block recovery. It clears only the marker; interrupted/failed evidence stays unusable until a new run passes.
Never manually clean the shared checkout or remove its marker to bypass this check.

Release subprocesses also hold the inherited lock, including remote Git, preflight, packaging and GitHub commands.
A killed parent cannot free the checkout while a child retains the lock. Docker containers are outside that lock;
the E2E harness removes its own stack.

### Manual Linux coverage

Actions → CI → Run workflow runs `python3 scripts/gate.py checks --scope full` with PostgreSQL. It creates no local
evidence and publishes nothing. A local macOS pass does not establish Linux coverage.

The gate is local workflow control. Another clone, old hook, forged record or `--no-verify` can bypass it; it provides
no server enforcement, signature or hermetic-build guarantee.
