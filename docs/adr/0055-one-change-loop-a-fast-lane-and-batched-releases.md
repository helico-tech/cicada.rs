# 0055. One change loop, a fast lane and batched releases

Date: 2026-11-22

## Status

Accepted for lighter-loops. Supersedes three parts of [ADR 0030](0030-three-loops.md): design, build and review in
three sessions of their own; every landed change a release; and the full loop for every change. Its lighter-work
rules stand: small changes, targeted checks, reviews that read, and one design review.
Partly superseded by [ADR 0073](0073-two-builders-in-the-change-loop.md): two builders when their changes share no crate.

## Context

The process costs more than the code. From 15 to 22 November `main` got 13 releases. `node-view-rustdoc`, a 30-line
rustdoc fix, took 6 hours from proposal to release: a design with a review, a build with a review, the full gate with
E2E, and a release with its pin refresh and docs merge. The owner kept two or three sessions going, and a review
request waited in `.reviews/` until someone opened a review session. The pre-push hook (ADR 0046) asks E2E only for a
version tag, so a landed change can be pushed to `main` without a release.

## Decision

- **One session, the change loop** (`/change-loop`). It describes a change with the owner, starts a builder subagent
  in a worktree in the background, lands and releases. While a builder runs, it describes the next change; at most two
  ready changes wait.
- **Each review is a fresh reviewer subagent** that takes a request by its id (`scripts/review.py take <id>`) and sees
  only the files and its brief. `scripts/review.py` and `/review-loop` stay, so another harness can still take a request.
- **A fast lane** for rustdoc and comments, tests only, and internal refactors. Never for a change to the public API,
  a wire format, the schema, a guarantee, the command line, an actor contract or the docs ownership rules. Its
  proposal says `Lane: fast` in a `## Lane` section, and the change loop checks it against this list. It has a
  proposal, tasks, a docs handoff and `skip_specs: true` in `.openspec.yaml`; no design, no spec deltas, no design
  review. The code review stays.
- **Landing is not releasing.** A change lands by a merge, its archive and a push of `main` with the full checks. It
  waits under the `unreleased` heading of `CHANGELOG.md`, with the other landed changes of the batch.
- **A batch is released** when no change is ready or in build, the changelog says `unreleased` and the batch holds a
  change outside the fast lane; at once when a landed change's proposal says `Release: now`; or when the owner asks.
  The release runs the E2E suite; builders run it only when a change's tasks ask for it, as they do for a change to
  the sample service, the E2E suite, `serve`/`worker` or the HTTP crates.
- **An E2E failure at a release stops landing** until a fix, first in the line with `Release: now`, or a revert lets
  the release pass.
- **One version per batch.** The first change after a release sets the next patch version. A breaking change raises
  an unreleased patch version to the next minor.
- `scripts/status.py` names the unreleased version and says when to release.
- Every authority that the specs and ADRs (ADR 0046 among them) give the design loop or the build loop belongs to
  the change loop. Their wording is fixed when a later change touches them.

## Consequences

- The owner talks to one session. Reviews run on the session's harness by default; another harness reviews only when
  the owner starts one.
- Fewer releases, fewer E2E runs and fewer pin refreshes. A fix reaches consumers when its batch is released, unless
  its proposal says `Release: now`.
- An E2E failure at a release may come from any change in the batch.
- A fast-lane change is described in its proposal and tasks only, so the code review is the one check of its intent.
- `main` holds landed, unreleased work between tags. Consumers pin tags, so they do not see it.
- The docs session stays a session of its own (ADR 0045).
