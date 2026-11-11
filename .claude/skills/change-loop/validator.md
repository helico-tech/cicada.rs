# Brief: validate a built change

A builder says it has built one OpenSpec change of this repository on the branch `feat/<change>`. The caller names the
change. Check that the work is right and complete. Trust nothing in the builder's report: run things yourself.

You work in your own worktree, so you test what is committed, on a clean checkout.

## Start

1. `pwd` must be a folder under `.claude/worktrees/`. If it is not, stop and report; do not work anywhere else.
2. `git reset --hard feat/<change>` puts your worktree on the builder's commits. In a later round, run it again: the
   builder may have rebased.
3. `git status --short` must print nothing.

## Read

- `AGENTS.md`.
- The change in `openspec/changes/<change>/`: proposal, design, specs and tasks.
- `git log --oneline main..HEAD` and `git diff main...HEAD`.

## Check

1. **Tasks.** Every task is ticked. For each one, the verification it names exists and passes: run the named tests.
2. **Spec.** Every scenario in the change's specs is covered by a test. Name the test for each scenario.
3. **The finishing checks, run fresh:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
   `cargo test --workspace`, `openspec validate <change> --strict`, and the PostgreSQL tests with
   `CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada`. If nothing answers on port 55432, say so; do not
   start containers.
4. **The diff.** Bugs. Behaviour the design does not describe. Anything outside the scope of the change. The rules of
   `AGENTS.md`: a schema change is an appended migration, public items have doc comments, the doctests of the README
   and the operations guide compile, a changed guarantee or wire format has its ADR. The docs and `CHANGELOG.md` say
   what changed, and the version in `Cargo.toml` is the one the caller named.
5. **The builder's example and concepts**, when the caller passes them on: they are true to the code.

## Rules

- Do not edit, commit or fix anything. Building and testing may write `target/`.
- A check you could not run is "not run", never "passed".
- Never push, merge, tag or archive.

## Report

At most 600 words:

1. Verdict in one line: pass, or fix needed.
2. Blocking findings, numbered. For each: file and line, what is wrong, how to see it (the command and its output),
   and what "fixed" looks like.
3. Notes that do not block.
4. The checks you ran with their results in numbers, and the checks you did not run with the reason.
