---
name: change-loop
description: Start or resume the change loop of this repository (ADR 0055, ADR 0073). One session describes the next OpenSpec change with the owner, has up to two builders (a Claude subagent, and GPT on Codex) build changes that share no crate, each in its own worktree, has a fresh reviewer of the other model review each change through scripts/review.py, then lands the changes on main one at a time and releases the batch when it is time.
disable-model-invocation: true
---

# The change loop

One session for the whole flow (ADR 0055, superseding parts of ADR 0030; two builders by ADR 0073). You describe
changes with the owner, start a builder for each ready change (at most two at once), a fresh reviewer of the other
model for each review, land each change on `main` and release the batch when it is time. The owner talks to you
only. The docs session stays a session of its own (ADR 0045).

You orchestrate: you describe, start agents, read reports, talk with the owner, land and release. You do not build or
review yourself.

## Start

Every time the skill is invoked, and again after a change is ready, has landed or is released:

1. `git fetch origin`. If `origin/main` is ahead of `main`, tell the owner before anything is described or built on
   the old base.
2. Run `python3 scripts/status.py`, `python3 scripts/review.py list`, `git status --short` and `git worktree list`.
   The main checkout is on `main`. If the status lists problems, solve them with the owner first.
3. Take the next step from the status, first that applies:
   - `release:` says to release: go to Release.
   - `build:` names a ready change and no builder runs for it: go to Build. It names up to two changes.
   - `describe:` names a change: go to Describe. It names none while two ready changes wait.
   - Otherwise wait for the builder or reviewer that runs, and tell the owner in one line.

While builders or reviewers run, go on describing the next change. Agent reports come in between: handle each when
it arrives.

## Small changes

A change is small: one behaviour, at most 5 tasks plus the finishing checks, a design of about 80 lines. Check this
before anything else. A bigger proposal is split into a line of small changes: propose the split to the owner, each
piece with one line on what it does, and on a yes write their proposals, each with an `Order` section. Then describe
the first piece.

## Describe

First choose the lane. The **fast lane** is a closed list: rustdoc and comments, tests only, and internal refactors.
Never a change to the public API, a wire format, the schema, a guarantee, the command line, an actor contract or the
docs ownership rules. A change on the list gets a `## Lane` section in its proposal with the line `Lane: fast`; check
the proposal against the list before you write it. Anything else is the normal lane, and needs no `## Lane` section.
A landed change that must reach consumers at once (a security fix, a bug a consumer hits) gets the line
`Release: now` in the same section, in either lane.

### Normal lane

1. **Read** the proposal, the ADRs it names, the code it touches, and the proposals before and after it. Documents
   written ahead of their turn (a design, a spec, `draft-tasks.md`) are a draft: go through them with the owner.
2. **Think it through with the owner**, a few topics per message. Start from the proposal's "To decide" list and add
   what you found yourself, and the crates the change touches. For each decision:
   - the options, one line each: what it adds and what it costs;
   - a small example per option where it helps: the code a user writes, the request, the table;
   - your pick, why, and why not the others. Say so when something is more than the problem needs.

   Label every question, yes/no ones too, with labels that are unique in the message, and end with the labels and
   your picks in one line. Write each decision into `design.md` when it is made, so that a new session can go on.
3. **Write** the rest of `design.md` (the decisions, each with its reason and the options not taken in a line) and the
   spec deltas in `specs/<capability>/spec.md`. Name the doc blocks, links, guarantee rows and skill routes the change
   breaks: the builder cannot fix them, as the docs session owns that prose (AGENTS.md, Documentation ownership).
   `openspec validate <change> --strict` passes. No `tasks.md` yet. A GUI change follows GUI changes below.
4. **Design review.** `python3 scripts/review.py request design <change>` prints the id. Start a reviewer for it
   (Review). One round; a second only when the design changed much.
5. **Bring the findings to the owner**, each with your own view. Decide together, and update the design and the spec.
6. **Tasks**, when the owner agrees: go to Tasks.

### Fast lane

1. **Read** the proposal and the code it touches. Write `## Lane` with `Lane: fast` into the proposal, and
   `.openspec.yaml` with `skip_specs: true` (next to `schema: spec-driven` and `created:`). No `design.md`, no spec
   deltas, no design review. Ask the owner only what the proposal leaves open.
2. **Tasks**: go to Tasks.

### GUI changes

A change that adds or changes a screen (the tray menu in a state, the Settings window, an inspector page, a sheet) is
built and reviewed against the owner's mockup boards ([MOCK-RULE]):

- **Screens.** Its design names each such screen with its board, or "no board". A screen without a board is built only
  when the owner has one drawn first, or agrees in the design to build it in a named board's style.
- **Copies.** You copy each board (the artboard's `.dc.html`, read with the Artifact tool) into
  `openspec/changes/<change>/mockup/`, with `mockup/SOURCE.md` naming the artifact's URL, its version and the date of
  the copy (as in `tray-menu-mockup`). Builders have no Artifact tool; they read the files. Only the board's HTML
  counts; a board without dark colours is named so in `SOURCE.md`, and the system's dark colours are then no departure.
- **Departures.** The design has a section `Departures from the board`: each place where a screen will differ from its
  board or add what the board does not show (a native control that cannot look the same, data the API lacks, an extra
  control, section or state), with its reason, for the owner to agree. Nothing else departs. Sample data and the
  board's desktop background are not departures.
- **Tasks.** The task that builds a screen says "match board X". The manual check list starts with "next to board X,
  light and dark"; builders never drive the GUI.

### Tasks

Write `tasks.md` by the rules in `openspec/config.yaml`; the last task runs the finishing checks. Check that the
proposal's `## Impact` has its `Crates:` line (the crates the tasks touch, or `Crates: none` for process and docs
changes): `status.py` pairs builds by it, and builds a change without it only alone. A change that touches
`sample-service/`, `e2e/`, the `serve` or `worker` commands of `cicada-app`, or the HTTP crates (`cicada-axum`,
`cicada-client`) asks for `python3 scripts/gate.py run --e2e` in its last task; any other change runs the gate without
E2E. A task never asks the builder to edit `README.md`, `AGENTS.md` or the handwritten docs: it asks for that content
in `docs-handoff.md` and keeps the checks; a block the design names as broken gets a fix prepared by the docs session
before landing. `openspec validate <change> --strict` passes. Commit on `main` as `Ready <change>: <what it does>`:
only the change's folder, and its ADR with its row in `docs/adr/README.md` if it has one. Stage by path, never
`git add -A` or `git commit -a`. Go to Start.

## Build

At most two builders at once (ADR 0073), for the changes the `build:` line names: those in build, and for a free
slot the first ready change in the order whose `Crates:` line meets no crate of a change in build. Shared workspace
files (`Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, the ADR index, the skills' versions) do not count: they are merged
at landing. A change without a `Crates:` line is built only alone. Never a third builder. At most two ready changes
wait.

Which builder: of the two changes in build, the earlier in the order gets Claude and the later gets GPT, unless the
owner says otherwise. Keep a note of who builds what: the review needs it.

1. **Earlier work.** If `git worktree list` shows the branch `feat/<change>` and you hold no agent for it, it is left
   from another session. A clean tree: `git worktree remove <path>`; the branch stays and the new builder picks it up.
   A dirty tree: ask the owner whether to continue it or drop it. To continue it, start the builder without isolation
   and add `Worktree: <path>` to its prompt; a worktree that `delegate_task` made (under `~/.t3/worktrees/`) counts as
   one of the builder's, for Claude and Codex alike.
2. **Version.** The `release:` line of `python3 scripts/status.py` names an `unreleased` version: that is the version,
   as the batch collects under one version. With nothing unreleased, raise the patch number of `Cargo.toml`, or the
   minor number when the proposal marks something **BREAKING**. A **BREAKING** change while the unreleased version is
   a patch raises it to the next minor: tell the builder that it renames the heading and rewrites the section's summary
   for the whole batch. A change that touches no crate (process or docs only) gets `none`.
3. **Builder.** The same prompt for both:

   > Read `<main checkout>/.claude/skills/change-loop/builder.md` and follow it. Change: `<change>`. Version: `X.Y.Z`.
   > Main checkout: `<main checkout>`.

   For a GUI change add: `Boards: openspec/changes/<change>/mockup/` and the board of each screen.

   - **Claude:** the Agent tool in the background: `subagent_type: "general-purpose"`, `model: "opus"`,
     `effort: "high"`, `isolation: "worktree"`, `run_in_background: true`.
   - **GPT:** in T3 Code, `delegate_task` with `isolation: "worktree"` and
     `target: { providerInstanceId: "codex", model: "gpt-6.2-sol", options: { reasoningEffort: "high" } }`, as the
     harness's rules require for another provider (call `orchestrator_capabilities` once per session first). It works
     on the branch `feat/<change>`, so `status.py` finds its worktree. Outside T3 Code, start it the way that harness
     starts an agent on another provider in its own worktree.
   - **Codex unavailable:** when `delegate_task` to Codex fails, build the second change with a Claude subagent as
     above, and tell the owner in one line.

   Keep its id, tell the owner in one line, and go on with Describe. The two builders share the gate's lock: each
   retries while the other's run holds it (builder.md, Gate evidence).
4. **When the builder reports.** A question about the design, or a departure from a board, goes to the owner; send the answer with SendMessage.
   `Docs: blocked` goes to Blocked on docs. Otherwise write `Version: X.Y.Z` and the builder's report to a file in your
   scratchpad, and ask for a review:

   ```sh
   python3 scripts/review.py request code <change> --head feat/<change> --built-by <builder's model> \
       --note-file <that file>
   ```

   `--built-by` names the builder's model, starting with its family: `claude-opus-5-5` or `gpt-6.2-sol`. Start a
   reviewer of the other family for the id it prints (Review).
5. **When the code review reports.**
   - `Verdict: pass`: go to Present.
   - `Verdict: fix needed`: check each blocking finding against the code. Drop one only when it is plainly wrong, and
     tell the owner why. Send the rest to the builder. When it reports, ask for round 2 the same way: `request code`
     again numbers the round and names the previous head. One fix round: when round 2 still blocks, take what is stuck
     to the owner.
   - Notes do not go to the builder; they go to the owner in Present.

## Review

Every review, design or code, fast lane too, is a fresh reviewer. It sees only the files and its brief, not your
reasoning. A code review goes to the model other than the builder's (ADR 0073): a change built by Claude is reviewed by
GPT, one built by GPT by Claude. A design review has no builder: either model may take it, Claude by default. Prompt,
with `<name>` the reviewer's model (`claude-opus-5-5` or `gpt-6.2-sol`; `review.py` takes the family from its start):

> Work in `<main checkout>`. Run `python3 scripts/review.py take <id> --by "<name>"`, then follow the brief it names
> (`.claude/skills/review-loop/<kind>.md`) and hand in the report as it says. Do not edit, commit or push anything.

- **Claude reviewer:** the Agent tool in the background: `subagent_type: "general-purpose"`, `model: "opus"`, default
  effort, no isolation, `run_in_background: true`.
- **GPT reviewer:** a new `delegate_task` on Codex, the same target as the GPT builder, without a worktree. Never a
  follow-up message to an earlier reviewer or builder thread.
- **Codex unavailable** for the whole request: a fresh Claude reviewer takes it with
  `--same-family "Codex unavailable"` added to its `take`. `review.py` records it in the report; tell the owner.

Its final message is not the review: read the report in `.reviews/<id>.result.md` (`python3 scripts/review.py list`
shows the state). If the owner wants another harness to review, do not start a subagent: tell the owner the id, and
run `python3 scripts/review.py wait <id>` in the background.

## Blocked on docs

The docs session owns `README.md`, `AGENTS.md` and the handwritten docs under `docs/` (AGENTS.md, Documentation
ownership; `protected()` in `scripts/docs-maintenance.py`). A builder never edits them; it writes what users must learn
into `openspec/changes/<change>/docs-handoff.md`. Most changes land with that handoff alone, and the docs session
writes the prose after they land. When a doctest, link, guarantee row or skill route cannot pass without new prose,
the builder reports `Docs: blocked`, and nothing lands until the prose is in the branch:

1. Write down the head (`git rev-parse feat/<change>`), the handoff and the paths of its Blocking items. Tell the owner,
   labelled: the docs session must prepare those paths on its branch against that head and current `main`, as whole
   files that keep its other edits, and name its commit and paths when done. It has no scan for unlanded branches yet,
   so ask explicitly. While nobody prepares them, the build stays blocked: no waiting it out, no disabled check, no
   hook bypass, no review.
2. When the commit comes, check it: `git diff <commit>^ <commit> --stat` and the diff itself hold only the named docs,
   no code, and both sides of a rename or deletion. Send the builder the commit and paths. It takes exactly those
   files into its branch without a merge, folds them into the commit that breaks the block, runs the blocked and the
   finishing checks and reports `Docs: owner fixes integrated`. Then ask for a review as in Build, step 4.
3. After every fix round or rebase, a clean one too, check again that each integrated path equals the docs branch
   (`git diff --quiet docs feat/<change> -- <paths>`) and that the prose still describes the code. Then the builder
   runs the affected checks in its worktree on the combined result, also when nothing changed:
   `cargo test -p cicada-examples --doc`, `--test docs` and `--test skill`. Unchanged prose can stop compiling against
   a new base. Only a changed path or behaviour needs a fresh preparation by the docs session first. A failing check
   sends the build back to step 1. A conflict in a protected file is never solved by taking one side: the docs session
   reconciles it.

## Present

Tell the owner, in this shape:

- What is new, in two or three sentences, and a small example: the builder's, at most 20 lines.
- New concepts and how each works. Leave this out when there are none.
- Results: tasks done, checks with their numbers, checks not run, what the review found and what was fixed.
- Docs: what the handoff asks of the docs session, and whether it is only handed off (the prose comes after landing)
  or owner fixes are integrated (name them, and the prose still to write). Never call handed-off prose written.
- The notes of the review, each with your pick: drop it, or a line in the later proposal that owns the area.
- Breaking changes, the version, the lane, and `Release: now` if the proposal says it.
- The question, labelled: "N. Land `<change>`? yes / no". With a standing yes from the owner, skip the question and
  land.

## Land

Only on the owner's yes. A no goes back to the builder, or to Describe. Landing is not releasing. One change lands
at a time, in whichever order the changes pass review; `status.py` resolves an `Order` that names an archived change
through that change's own `Order`, so the open changes stay one line.

1. `git fetch origin`. If `main` moved: `git -C <builder worktree> rebase main`. On a conflict,
   `git -C <builder worktree> rebase --abort`, send the builder to rebase and resolve, and ask for one more review
   round. When the other change landed first, check the ADR number of this change: if `main` took it meanwhile, the
   builder renumbers the ADR, its row and the references to it, in the branch. A branch with owner fixes integrated is
   checked again after any rebase, also a clean one: you check the paths, and the builder runs the affected checks in
   its worktree on the rebased branch (Blocked on docs, step 3). Merge only when they pass.
   A GUI change: read each board's live version with the Artifact tool and compare it with the copy in `mockup/`. If
   one changed, the standing yes does not cover it: the owner accepts the copied version, or the builder takes the new
   copy (sent as a file from your scratchpad), updates the screen, and a new code review round checks it.
2. `git merge --ff-only feat/<change>` on `main`.
3. Move the change's reviews into its folder, so that they are archived with it:
   `mkdir -p openspec/changes/<change>/reviews && mv .reviews/<change>.* openspec/changes/<change>/reviews/`.
4. `openspec archive <change> -y`, with `--skip-specs` when its `.openspec.yaml` says `skip_specs: true`. A new spec in
   `openspec/specs/` gets one or two real sentences under `## Purpose`. Commit `Archive <change>`, staged by path:
   `git add -A -- openspec/changes/<change> openspec/changes/archive openspec/specs`.
5. In the main checkout, `python3 scripts/gate.py run` (the full checks on the new tip, about 7 minutes), then
   `git push origin main`. One gate run at a time: while a builder's run holds the lock, `gate.py run` ends at once
   with `busy`. Retry in short tries for at most 50 minutes; each try is a background Bash command with
   `timeout: 7200000`, as the try that gets the lock is the run. Never stop a run once it started. The pre-push hook
   asks E2E only for a version tag. An abandoned gate run (`python3 scripts/gate.py status` says so) goes to the
   owner before `recover`. The pre-push hook is installed once per repository, with the owner's yes:
   `python3 scripts/gate.py install-hook`.
6. Clean up: `git worktree remove` the builder's worktree, `git branch -d feat/<change>`, delete the leftover
   `worktree-agent-…` branches, `git worktree prune`.
7. Tell the owner in two lines, with the path of the archived handoff
   (`openspec/changes/archive/<date>-<change>/docs-handoff.md`): the docs session may write its prose on `main` now;
   the installation pins change only after the release. Go to Start.

## Release

The `release:` line of `python3 scripts/status.py` says when: at once when a landed change's proposal says
`Release: now`; otherwise when no change is ready or in build and the batch holds a change outside the fast lane. A
batch of fast-lane changes alone waits for the next behaviour change, or for the owner to ask. A batch with a GUI
change waits until the owner has seen each screen next to its board, light and dark. Ask the owner before a release
unless they gave a standing yes.

1. `git fetch origin`; `main` equals `origin/main`.
2. As in `docs/how-to/releasing.md`: the README tag lines and the changelog date (the `unreleased` heading of the batch
   gets today's date) in one commit `Release X.Y.Z: …` on `main`, staged by path: `git add -- README.md CHANGELOG.md`.
3. `python3 scripts/gate.py release vX.Y.Z` in the main checkout (ADR 0046). It checks the version and dated notes,
   reuses or runs the gate evidence and the E2E suite (retry its `busy` like Land, step 5), pushes `main` without force,
   tags, packages the skill, pushes the tag and publishes a verified draft Release. If it fails before the tag push, no
   tag exists: tell the owner. If it fails later, run it again: it resumes the same commit and tag object; never delete
   or move a pushed version tag.
4. **An E2E failure stops landing.** Nothing else lands until the release passes. Find the cause by running the E2E
   suite on the batch's commits (`CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture` in a detached worktree under
   `.claude/worktrees/`). The fix goes first in the line as a change of its own with `Release: now` (fast lane when it
   qualifies), or as a revert of the change that broke it. Tell the owner what you found.
5. Tell the owner in one line: the version and the changes it holds. The docs session refreshes the installation pins
   from the release, and takes the version from the release, not from a handoff. Go to Start.

## Rules

- The owner's yes in Present covers the merge, the archive, the gate run and the push of `main` for that one change.
  A release needs its own yes, unless the owner gave a standing yes. Nothing else is pushed, merged or tagged.
- Builders and reviewers never push, merge, tag or archive.
- One change at a time is described, at most two builders run (their changes share no crate), a change is reviewed by
  the model other than its builder's, and never more than two ready changes wait.
- While a change is built, only its builder edits its files, in its branch. Do not edit them on `main`.
- Stage by path in the main checkout, never `git add -A` or `git commit -a`.
- Read an agent's worktree with `git -C <path>` and absolute paths; never `cd` into it.
- Tell the owner in one line when an agent or a review starts and when it reports. Never state a result before its
  report has arrived.
- An idea or a finding outside the change: a line in the later proposal that owns the area, or, when it blocks, a new
  change with only a `proposal.md` and an `Order` section. Ask the owner.
- When the session ends, name each worktree: kept because a build runs, or removed.
