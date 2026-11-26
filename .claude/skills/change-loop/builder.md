# Brief: build a ready change

You build one small OpenSpec change of this repository, completely, in your own git worktree. The caller names the
change, its version, the path of the main checkout, and maybe a worktree. The design of the change is agreed: build
what it says. You do not change the design. Work fast: the owner wants short iterations.

## Start

1. If the caller names a worktree, `cd` into it. `pwd` must be a folder under `.claude/worktrees/`, or a worktree that
   another harness's tool (`delegate_task` in T3 Code) made, such as one under `~/.t3/worktrees/`. If it is neither,
   stop and report; do not work anywhere else. A builder started by such a tool works in the worktree it made. Another
   builder may run next to you on another change (ADR 0073): never touch its worktree or branch.
2. Get onto the right commits and branch. The ready change is committed on the local branch `main`, which your
   worktree may not start from:

   ```sh
   git show-ref --verify --quiet refs/heads/feat/<change> && git merge --ff-only feat/<change>   # earlier work, if any
   git rebase --autostash main   # keeps the uncommitted work of a resumed worktree
   git branch -M feat/<change>
   ```

3. Read `openspec/changes/<change>/` (proposal, design, specs, tasks), `AGENTS.md`, and the parts of
   `docs/architecture.md` and the ADRs that the change names. Read Documentation below before the first task.

## Build

- Work through `tasks.md` in order. Tick a task only after the verification it names has passed. If some tasks are
  ticked already, check that their work is there and go on with the rest.
- While you work, run only what a task touches: `cargo test -p <crate> <filter>` and
  `cargo clippy -p <crate> --all-targets`. Run the whole workspace once, in the last task.
- Commit after each task or group of tasks. Write the message like the ones in `git log`: one sentence that says what
  the commit does. Every commit builds.
- **Version.** The caller gives `X.Y.Z`, or `none` for a change that touches no crate: then leave every version place
  and `CHANGELOG.md` alone. Landed changes collect under one version until a release (ADR 0055). When the top heading
  of `CHANGELOG.md` is already `## X.Y.Z — unreleased`, the version places hold it: add this change's entries under
  that heading. Otherwise set `X.Y.Z` in `[workspace.package]` of `Cargo.toml`, run `cargo check` (it updates
  `Cargo.lock`), set `metadata.cicada-version` in every `skills/*/SKILL.md` (now `cicada` and `actor-design`) and
  `version` in `.claude-plugin/plugin.json`, regenerate `docs/openapi.json` (`CICADA_UPDATE_OPENAPI=true`), and put
  the entries of this change under `## X.Y.Z — unreleased` in `CHANGELOG.md`. A **BREAKING** change while the
  unreleased version is a patch: the caller gives the next minor; set it in every place above, rename the unreleased
  heading to it, and rewrite the section's summary for the whole batch. Leave the `tag = "v…"` lines of the README
  alone: they change at the release.
- **PostgreSQL.** Its tests need the database on port 55432
  (`CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada`). If nothing answers there, run
  `docker compose -f <main checkout>/docker-compose.yml up -d postgres`. If Docker is not running, go on without it and
  say so in the report.
- The last task runs the finishing checks. All of them pass before you report the build as done; a check that waits
  for protected prose is reported as blocked (Documentation).
- **Gate evidence** (ADR 0046). Commit first, then run `python3 scripts/gate.py run` in your worktree (the full
  policy, every former CI check with PostgreSQL, about 7 minutes, longer under load). Only when the tasks ask for E2E,
  follow it with `python3 scripts/gate.py run --e2e` the same way (it reuses that evidence and adds the Docker E2E
  suite, about 4 minutes); otherwise E2E runs at the release (ADR 0055). The gate checks your committed head in its own
  worktree and records evidence that the change loop's push and release reuse. Uncommitted edits are not checked.
  - **One run at a time** (ADR 0073). While another run holds the lock (the other builder's, or the session's),
    `gate.py run` ends at once with `busy`. Retry in short tries, a minute or so apart, for at most 50 minutes; then
    report that the gate stayed busy. The try that gets the lock is the run, so start every try as a background
    command with a 2-hour limit (in Claude Code: Bash with `run_in_background: true` and `timeout: 7200000`).
  - Keep your turn open until the run ends, waiting on that command: a run still going when your turn ends is killed
    and leaves a lock only the owner may clear. Never stop a run once it started.
  - Report its final status lines (`python3 scripts/gate.py status`). A failure is a failing check: fix the cause,
    commit, run it again. An abandoned-run message is for the owner: report it, do not run `recover` yourself. Never
    install the hook, push or publish.

## Documentation

The docs session owns `README.md`, `AGENTS.md` and the handwritten docs under `docs/`: everything there except
`docs/adr/`, `docs/reference/contracts/`, `docs/reference/cli.md` and `docs/openapi.json` (AGENTS.md, Documentation
ownership). A local hook refuses your commits of the README and `docs/` part (`protected()` in
`scripts/docs-maintenance.py`). You own the code, its doc comments, the worked examples, the consumer skill's routes and
version, ADRs, the generated references, `CHANGELOG.md` and the change's own files.

- Never edit a protected file, not even for a moment to try a text, and never bypass the hook (`--no-verify`). This
  rule wins over a task: a task that asks for a guide, README, guarantees or AGENTS.md edit is done by writing that
  content into the handoff. Its verification stays: the checks it names must still pass.
- Every change writes `openspec/changes/<change>/docs-handoff.md` and commits it with the change, also when users see
  nothing new: then it says `None` and why, and still gives the version. Update it with every fix or rebase that
  changes what users see. It does not name its own commit: the review request's head does.
- **Blocking.** A doctest, link, guarantee row or skill route that fails because protected prose must change blocks
  the build. Finish the rest, leave the tasks with the failing checks unticked, write each failing check, its file and
  block, and your proposed replacement under Blocking in the handoff, commit, and report `Docs: blocked`. Do not
  disable the block, weaken the check or report the build as ready for review.
- **Owner-prepared fixes.** The caller may send a commit of the docs branch and its paths. Inspect its full diff: it
  holds only those docs, no code. Take exactly those paths into your branch without merging:
  `git checkout <commit> -- <paths>`, and `git rm` for each path the commit deletes (both sides of a rename). The hook
  accepts a staged file only when it equals the docs branch's current one; if it refuses, the branch moved: report it,
  do not edit the file. Fold the fix into the commit that breaks the block (amend it, or squash a fixup into that one
  commit only), so that no commit of the branch has the break. Then run the blocked checks and the finishing checks,
  tick the tasks, and report `Docs: owner fixes integrated`. After every later fix or rebase, a clean one too, run
  `cargo test -p cicada-examples --doc`, `--test docs` and `--test skill` again on the combined result. A fix or
  rebase that changes what that prose describes needs a fresh preparation first: report it; never resolve a conflict
  in a protected file yourself.

The handoff, in this shape (the archived flow changes hold real examples):

```markdown
# Docs handoff for <change>

Version: X.Y.Z, or none for a change that is not released. Refresh the dependency pins of `docs/how-to/install.md`
once it is published.

## What changed

<user-visible behaviour, the public items, and the tests and examples that prove it; or `None` and why>

## <n>. <short title>

**Where.** `<file>`, `<heading text>`: code spans, not links, as the heading may not exist yet.
**Text.** <the proposed substance, or agreed text in a quote; a Rust block names the doc example or the file of
`examples/` it is compiled from>
**Caveats.** <upgrade limits, timing, what users must not assume>

## Blocking

<`None`, or per item: the failing check, the file and block, the proposed replacement; once integrated, the owner's
commit and paths>

## Status

<the checks you ran and their results; what is proposed text, what the owner prepared and you integrated, and what
prose is not written yet>
```

## Rules

- Stay in your worktree. Never push, merge, tag or archive, and never touch another worktree or the main checkout.
- Build the design as written. If the design is wrong or unclear, or a task cannot be done as written: commit what
  works, stop, and report the question. Do not decide it yourself, and do not widen the scope.
- If the caller sends a decision that changes the design, the spec or the tasks, make that edit in your branch.
- Fix the cause of a failing test. Do not weaken a test or hide an error.
- Never report a check as passed that you did not run.

## Fix round

The caller may come back with the blocking findings of a review. Fix each one, or say why it is not a fault. Update
the handoff when a fix changes what users see. Run the tests the fixes touch, then `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` once. Commit, run the gate on the
new head as in Gate evidence, and report again.

## Report

At most 300 words:

1. Each task: done, or not done and why.
2. The checks you ran, with results in numbers (tests passed and failed), and the checks you did not run, with the
   reason.
3. Where you departed from the design, and your open questions.
4. An example for the owner: at most 20 lines of code, a command or a request that shows what is new. Take it from a
   test or a doc block in your branch, so that it is true.
5. New concepts: each new term or idea that a user of the library meets, with two or three plain sentences on how it
   works. "None" when there are none.
6. The breaking changes for users of the library.
7. Docs: `handed off, no blockers`; `blocked`, with the failing checks, the paths and your head (`git rev-parse HEAD`);
   or `owner fixes integrated`, with the owner's commit and paths and the prose still to write.
