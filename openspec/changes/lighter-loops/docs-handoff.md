# Docs handoff for lighter-loops

Version: none. This change touches no crate, skill or wire format, so it lands without a version bump, changelog entry
or release, and the installation pins stay as they are.

## What changed

Users of the library see nothing new. The way this repository is worked on changes (ADR 0055):

- `/change-loop` (`.claude/skills/change-loop/SKILL.md`) replaces `/design-loop` and `/build-loop`. One session
  describes a change with the owner, starts a builder subagent in a worktree and a fresh reviewer subagent per review,
  lands and releases. `.claude/skills/design-loop/` and `.claude/skills/build-loop/` are gone.
- The builder's brief moved: `.claude/skills/build-loop/builder.md` is now `.claude/skills/change-loop/builder.md`.
  It runs `python3 scripts/gate.py run` without `--e2e` unless the tasks ask for it, and adds a change to an existing
  `unreleased` changelog section instead of starting a new version.
- A fast lane: rustdoc and comments, tests only, internal refactors, marked `Lane: fast` in a `## Lane` section of the
  proposal; no design, no spec deltas, no design review; the code review stays.
- Landing is not releasing: a landed change is pushed to `main` after the full gate and waits under the `unreleased`
  heading of `CHANGELOG.md`. A batch is released when no change is ready or in build and it holds a change outside the
  fast lane, at once when a landed proposal says `Release: now`, or when the owner asks. E2E runs at the release.
- `python3 scripts/review.py take <id>` claims exactly one request (proven by `scripts/test_review.py`).
  `python3 scripts/status.py` prints a `release:` line (proven by `scripts/test_status.py`).
- `/review-loop` stays, for another harness.

## 1. AGENTS.md: the builder's brief link

**Where.** `AGENTS.md`, `Rules`, the item `Documentation ownership.`
**Text.** Point the link to `.claude/skills/change-loop/builder.md`:
`(template in [.claude/skills/change-loop/builder.md](.claude/skills/change-loop/builder.md))`.
**Caveats.** This is the Blocking item below.

## 2. AGENTS.md: the map row of `.claude/skills/`

**Where.** `AGENTS.md`, `Map`, the row `.claude/skills/`.
**Text.** "The change loop (ADR 0055): `change-loop` with the builder's brief and docs handoff template, and
`review-loop` with the briefs of a design and a code review, for reviews in another harness."

## 3. AGENTS.md: releases and the flow

**Where.** `AGENTS.md`, `Rules`, the item that starts "Releases are tags on `main`", and `Specs and changes: OpenSpec`
(the line "Work on them goes through three loops (ADR 0030)." and the item `Flow: three sessions.`).
**Text.**
- Releases: "published by the change loop with `python3 scripts/gate.py release vX.Y.Z` on the owner's yes."
- Intro: "Work on them goes through one change loop (ADR 0055)."
- Flow: "One session, `/change-loop`, describes the next change with the owner, has a builder subagent build it in a
  worktree under `.claude/worktrees/` and a fresh reviewer subagent review it, lands it on `main` and releases the
  batch when it is time. Reviews go through `scripts/review.py`, which keeps requests in `.reviews/`; another harness
  can take one with `/review-loop` (`.claude/skills/review-loop/SKILL.md`). Each built change hands user prose to the
  docs session in `docs-handoff.md` (ADR 0045). A change is small (one behaviour, at most 5 tasks) and built through
  the change loop unless the owner says otherwise. A fast-lane change (rustdoc and comments, tests only, internal
  refactors; `Lane: fast` in the proposal) has no design or spec deltas. A landed implementation change waits under
  the `unreleased` heading of `CHANGELOG.md`; the change loop releases a batch when `scripts/status.py` says so
  (`release:` line); documentation-only maintenance is exempt. The rules for the artifacts are in
  `openspec/config.yaml`."
**Caveats.** Replaces "Every landed implementation change is a release".

## 4. docs/how-to/releasing.md

**Where.** `docs/how-to/releasing.md`: step 3 ("On the owner's approval in the build loop"), the Prerequisites
paragraph ("Only the owner-approved build loop installs the shared hook or publishes"), the paragraph "When the next
implementation change lands, add `## X.Y.Z — unreleased`", and the line "The build loop installs
`python3 scripts/gate.py install-hook`".
**Text.** Name the change loop instead of the build loop. Say when a release happens: a landed change no longer is a
release; landed changes collect under one `unreleased` version, the first change after a release sets the next patch
version and a breaking change raises an unreleased patch version to the next minor. The change loop releases a batch
when `python3 scripts/status.py` says `release now` (no change ready or in build and a change outside the fast lane
landed; or a landed proposal says `Release: now`), or when the owner asks. Landing pushes `main` after
`python3 scripts/gate.py run`; E2E runs only at the release. An E2E failure at a release stops landing until a fix
with `Release: now`, or a revert, lets the release pass.
**Caveats.** The release commit no longer carries the archive: the change loop archives and commits at landing.

## 5. docs/maintaining/maintenance.md

**Where.** `docs/maintaining/maintenance.md`, the line "The build loop reports its path under
`openspec/changes/archive/<date>-<change>/`."
**Text.** "The change loop reports its path under `openspec/changes/archive/<date>-<change>/` when the change lands."
Add: the docs session may write a landed change's prose on `main` before its release; installation pins change only
after the release. Take a release's version from the release, not from a handoff: a BREAKING change can raise it
after earlier handoffs of the batch were written.

## Blocking

1. `cargo test -p cicada-examples --test docs`, `the_links_between_the_docs_resolve`: `AGENTS.md:76: the link
   .claude/skills/build-loop/builder.md points to a file that does not exist`. File `AGENTS.md`, the
   `Documentation ownership.` item of `Rules`. Proposed replacement: item 1 above. Items 2 and 3 touch the same file
   and are best prepared with it.

## Status

- Proposed text: items 1 to 5. No prose is written; nothing is integrated.
- Ran: `cargo test -p cicada-examples --test docs` (10 passed, 1 failed: the Blocking link); the other results are in
  the builder's report.
