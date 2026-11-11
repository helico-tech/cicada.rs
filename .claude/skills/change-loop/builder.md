# Brief: build a ready change

You build one OpenSpec change of this repository, completely, in your own git worktree. The caller names the change,
its version and the path of the main checkout. The design of the change is agreed: build what it says. You do not
change the design.

## Start

1. `pwd` must be a folder under `.claude/worktrees/`. If it is not, stop and report; do not work anywhere else.
2. Get onto the right commits and branch. The ready change is committed on the local branch `main`, which your
   worktree may not start from:

   ```sh
   git show-ref --verify --quiet refs/heads/feat/<change> && git merge --ff-only feat/<change>   # earlier work, if any
   git rebase main
   git branch -M feat/<change>
   ```

3. `openspec instructions apply --change <change>` prints the files of the change and the project rules. Read them,
   `AGENTS.md`, `docs/architecture.md` and the ADRs the change names.

## Build

- Work through `openspec/changes/<change>/tasks.md` in order. Tick a task only after the verification it names has
  passed. If some tasks are ticked already, check that their work is there and go on with the rest.
- Commit after each group of tasks. Write the message like the ones in `git log`: one sentence that says what the
  commit does. Every commit builds.
- **Version.** The caller gives `X.Y.Z`. Set it in `[workspace.package]` of `Cargo.toml`, run `cargo check` (it
  updates `Cargo.lock`), and put the entries of this change under `## X.Y.Z — unreleased` in `CHANGELOG.md`. Leave the
  `tag = "v…"` lines of the README alone: they change at the release.
- **PostgreSQL.** Its tests need the database on port 55432
  (`CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada`). If nothing answers there, run
  `docker compose -f <main checkout>/docker-compose.yml up -d postgres`. If Docker is not running, go on without it and
  say so in the report.
- The last task runs the finishing checks. All of them pass before you report.

## Rules

- Stay in your worktree. Never push, merge, tag or archive, and never touch another worktree or the main checkout.
- Build the design as written. If the design is wrong or unclear, or a task cannot be done as written: commit what
  works, stop, and report the question. Do not decide it yourself, and do not widen the scope.
- While you build, you are the only one who edits the files of this change. If the caller sends a decision that
  changes the design, the spec or the tasks, make that edit in your branch.
- Fix the cause of a failing test. Do not weaken a test or hide an error.
- Never report a check as passed that you did not run.

## Fix rounds

The caller may come back with findings from a validator. Fix each one, or say why it is not a fault. Run the checks
the fix touches and the finishing checks again, commit, and report again.

## Report

At most 600 words:

1. Each task: done, or not done and why.
2. The checks you ran, with results in numbers (tests passed and failed), and the checks you did not run, with the
   reason.
3. Where you departed from the design, and your open questions.
4. An example for the owner: at most 20 lines of code, a command or a request that shows what is new. Take it from a
   test or a doc block in your branch, so that it is true.
5. New concepts: each new term or idea that a user of the library meets, with two or three plain sentences on how it
   works. "None" when there are none.
6. The breaking changes for users of the library.
