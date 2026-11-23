# Design

Decided with the owner, 2026-11-22 and 23. Evidence: `node-view-rustdoc`, a 30-line rustdoc fix, took 6 hours from
proposal to release (11-17, 13:16 to 19:19): a design with a review, a build with a review, the full gate with E2E and
a release. The first design review then waited in `.reviews/` with no review session open; it was withdrawn unread,
and decisions 12 to 14 came from that. A reviewer subagent reviewed the result (`lighter-loops.design.1`).

Two facts make this cheap. The pre-push hook (ADR 0046) asks E2E only for a version tag; pushing `main` needs the
full checks alone. And OpenSpec reads `skip_specs: true` in a change's `.openspec.yaml`: such a change passes
`openspec validate --strict` and is archived without spec deltas.

## Decisions

1. **One change, not two or three.** One ADR replaces three parts of ADR 0030, and all of it edits the same loop
   skills; it fits in 5 tasks. Not split: several ADRs and rounds over the same files is the overhead this removes.
2. **The fast lane is a closed list.** Rustdoc and comments, tests only, internal refactors. Never a change to the
   public API, a wire format, the schema, a guarantee, the command line, an actor contract or the docs ownership
   rules. The proposal says `Lane: fast`; the change loop checks it against the list. Not "anything without a public
   API change": a behaviour change needs a design. Not "the owner decides each time": the rule saves a question.
3. **A fast-lane change has** `proposal.md`, `tasks.md`, `docs-handoff.md` and `.openspec.yaml` with
   `skip_specs: true`; no `design.md`, no spec deltas, no design review. `scripts/status.py` already reads a change
   with a `tasks.md` as ready. Not one combined file: `status.py`, the builder's brief and the archive would change.
4. **A fast-lane change keeps the code review.** It is the one safety net left, and a small diff reads in minutes.
   Not no review: a refactor can change behaviour unseen.
5. **A batch is released** when no change is ready or in build and the top heading of `CHANGELOG.md` says
   `unreleased`, provided the batch holds a change outside the fast lane. It is released at once when a landed
   change's proposal says `Release: now` (a security fix, a bug a consumer hits), or when the owner asks. A batch of
   fast-lane changes alone waits for the next behaviour change or the owner. Not the owner's word only: releases get
   forgotten. Not every N changes or once a day: arbitrary.
6. **Landing pushes `main`; E2E runs at the release.** To land: merge, archive, a full gate run on the new tip (the
   archive moves files that are not Markdown, about 7 minutes), push `main`. To release: date the changelog, set the
   README pins, `gate.py release` with E2E. The docs session may write a landed change's prose on `main` before its
   release; installation pins change only after it. Not keeping landed changes on the local `main`: nobody else sees
   them.
7. **A batch collects under one version.** An `unreleased` changelog heading is the version, so the first change after
   a release sets the next patch version and later ones add entries under it. A **BREAKING** change that lands while
   the unreleased version is a patch raises it to the next minor; its builder sets every version place and the
   heading, and rewrites the section's summary for the whole batch. Not releasing the batch first: one more release
   for a tidier number.
8. **The builder runs `gate.py run`, not `--e2e`,** unless its tasks say so. The change loop writes E2E into the tasks
   of a change that touches `sample-service/`, `e2e/`, the `serve` or `worker` commands of `cicada-app`, or the HTTP
   crates (`cicada-axum`, `cicada-client`). Not E2E per change: every change pays for what the release checks anyway.
9. **`scripts/status.py` prints a `release:` line**, e.g. `release: 0.20.6 unreleased, 2 changes landed`, and says to
   release when decision 5 holds. It reads the top heading of `CHANGELOG.md` and the lanes of the changes archived
   since the last tag. A new `scripts/test_status.py` covers it. Not leaving it to the loop to read the changelog: the
   loop takes its next step from `status.py`.
10. **`Lane: fast` and `Release: now` are lines in a `## Lane` section of the proposal.** The change loop sets the
    lane when it describes a change and reads `Release: now` when it lands one. `openspec/config.yaml` names the
    section. A proposal without it is in the normal lane. Not fields in `.openspec.yaml`: that file is OpenSpec's.
11. **ADR 0055 records this** and supersedes three parts of ADR 0030: the three sessions, "every landed change a
    release" and the full loop for every change. Its lighter-work rules stand. ADR 0030's status line and its row in
    `docs/adr/README.md` point to 0055. Every authority that specs and ADRs give the design loop or the build loop
    (`release-management` and `documentation` specs, ADR 0046) belongs to the change loop; their wording is fixed
    when a later change touches them. Not spec deltas now: long copies that change only a name.
12. **One session, the change loop.** `/change-loop` (`.claude/skills/change-loop/SKILL.md`) replaces `/design-loop`
    and `/build-loop`. It describes a change with the owner and writes its tasks, starts the builder subagent in a
    worktree in the background, lands and releases. While a builder runs, it describes the next change; still at most
    two ready changes wait. `builder.md` moves into its folder. Not starting a reviewer only when nobody takes a
    request: the owner still keeps two sessions. Not keeping the sessions: they cause the waits this removes.
13. **Each review is a fresh reviewer subagent** (Opus, default effort). The session asks with `scripts/review.py
    request`, then starts the reviewer with that id; it runs `review.py take <id>`, a new optional argument, and
    follows its brief. It sees only the files and its brief, not the session's reasoning. `review.py` and
    `/review-loop` stay: the reports are archived with the change, and another harness can still take a request.
    Not `take` without an id: with a design and a code review open at once, a reviewer would take the oldest.
14. **The owner talks to one session.** It asks the design questions and, unless the owner gave a standing yes, the
    question to land and release.
15. **An E2E failure at a release stops landing.** The change loop finds the cause by running the E2E suite on the
    batch's commits. The fix goes first in the line as a change of its own with `Release: now` (fast lane when it
    qualifies), or as a revert of the change that broke it. Nothing else lands until the release passes. Not E2E at
    every landing: it brings back the cost decision 8 removes.

## What it touches

- `.claude/skills/change-loop/SKILL.md`, new, made from the two skills it replaces; `.claude/skills/design-loop/`
  and `build-loop/` are removed.
- `.claude/skills/change-loop/builder.md`, moved from `build-loop/`: the gate without E2E, the version rule (7, 8).
- `.claude/skills/review-loop/`: `SKILL.md` says it is for another harness; `code.md` reads the proposal and tasks of
  a fast-lane change, which has no design (4, 13).
- `openspec/config.yaml`: the change loop, the `## Lane` section, the fast-lane artifacts and the E2E rule (3, 8, 10).
- `scripts/status.py`, its docstring, and a new `scripts/test_status.py` (9); `scripts/review.py take <id>` and its
  test (13).

## Docs it breaks

No doctest, guarantee row or skill route. One link breaks, so this change is blocked on docs until the docs session
prepares `AGENTS.md`:

- **Blocking:** `AGENTS.md` links `.claude/skills/build-loop/builder.md`, which moves to `change-loop/`.
- `AGENTS.md`: the three loops in "Specs and changes", the map row of `.claude/skills/`, and "Every landed
  implementation change is a release".
- `docs/how-to/releasing.md` (it names the build loop and says when a release happens) and
  `docs/maintaining/maintenance.md` (the build loop reports the archived handoff).
- For the docs session: take a release's version from the release, not from a handoff, since a BREAKING change can
  raise it after earlier handoffs of the batch were written.
