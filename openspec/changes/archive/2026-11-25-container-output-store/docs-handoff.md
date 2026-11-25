# Docs handoff for container-output-store

Version: 0.22.1 (a new unreleased batch after the 0.22.0 release). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` has a store for container outputs that the host provides, in `src/outputs/`:

- `OutputStore`, an async trait keyed by container name (`Arc<dyn OutputStore>`), with `log_end`, `append_logs`,
  `read_logs`, `put_file`, `files`, `read_file` and `remove`; the types `LogEnd { cursor, lines }`,
  `StoredLine { number, line }` and `StoredFile { path, size, mode }`; the error `OutputError { kind, message }` with
  `OutputErrorKind::{Invalid, Conflict, Unavailable, Other}` (`is_retryable` only for `Unavailable`).
- The input helpers `check_container_name` and `clean_output_path`, public, which every store applies first.
- Behind `testing`: `MemoryOutputStore` and `check_output_store(&store)`.
- Serde on `LogLine` and `LogStream`: `{"time": "<RFC 3339>", "stream": "out" | "err", "text": "…"}`.

Nothing writes to the store yet: the next changes of the line (`container-log-archive` first) do. Proven by the unit
tests of `crates/cicada-containers/src/outputs/` (`mod.rs`, `memory.rs`, `check.rs`): the check passes on the memory
store, also when it holds other containers, and panics naming the rule on four broken stores; the doc examples of
`check_container_name`, `clean_output_path` and `MemoryOutputStore` run.

## 1. A section on keeping outputs

**Where.** `docs/guide/containers.md`, a new section after `The container actor` and before
`Write or wrap a runtime`, such as `## Keep outputs`.
**Text.** A host keeps each container's log lines, log cursor and output files outside the container, in a store it
implements: `OutputStore`, keyed by the container's name, so one store serves every job (Factory keeps a folder per
run). The methods, one line each:

- `log_end(container)`: the stored cursor (`None` when nothing is stored) and the number of stored lines. A log view
  shows the last lines from it; a writer resumes from its cursor.
- `append_logs(container, after, page)`: stores the page's lines and its `next` cursor in one write.
- `read_logs(container, from, limit)`: up to `limit` lines from line number `from` (0-based, stored order), each with
  its number; none past the end. A host's HTTP route pages with it.
- `put_file(container, file)`: stores a `CopyFile` (as `copy_out` gives them), replacing one at the same path.
- `files(container)`: path, size and mode of each stored file, in path order.
- `read_file(container, path)`: the file or none.
- `remove(container)`: deletes lines, cursor and files; nothing stored is `Ok`. Afterwards the container starts
  afresh: its cursor is `None` again.

A block for the section: the doc example of `MemoryOutputStore` (`crates/cicada-containers/src/outputs/memory.rs`),
which runs as a doctest of the crate. It creates a container on `FakeRuntime`, sets its log, appends the page of
`logs_since` from `log_end`'s cursor, shows that a repeat of that append fails with `Conflict`, reads a line back by
number and runs `check_output_store`. Copy it as is (it needs the features `testing` of `cicada-containers`, which
`cicada-examples` turns on).

**Caveats.** Nothing in Cicada writes to the store in this version.

## 2. The one-write rule

**Where.** The new section of item 1, a paragraph or subsection such as `### One write for lines and cursor`.
**Text.** `append_logs(container, after, page)` stores the lines and the cursor together or not at all, and only when
the stored cursor equals `after` (`None` for a container with nothing stored). Otherwise it fails with `Conflict` and
stores nothing. So a writer that crashed after a write reads `log_end` again and goes on without a repeated or lost
line, and two writers of one container cannot interleave: the second gets `Conflict`. An empty page stores its cursor
alone. A store implements this as one transaction (a database) or one atomic replace (a folder).
**Caveats.** The store promises atomicity across a crash and the guard under writers in parallel (on other nodes too);
no test can prove those for a host's store.

## 3. The input rules

**Where.** The new section of item 1, such as `### Names and paths`.
**Text.** A container name and a file path can come from a user (a host's HTTP route), and a folder store turns them
into file paths. Every method checks them first and fails with `Invalid` before it touches anything:
`check_container_name` (a letter or digit first, then only `A-Za-z0-9_.-`: no `/`, never `..`) and
`clean_output_path` (absolute, no `..` part, not `/`; empty and `.` parts dropped, so `/a`, `//a` and `/./a` are one
file, stored as `/a`). A store of a host calls both and uses the cleaned path as its key.
**Caveats.** `read_file` and `files` give the cleaned path, not the one put.

## 4. Test a store

**Where.** `docs/guide/containers.md`, `Test with the fake`, a paragraph at the end, or the new section of item 1.
**Text.** `MemoryOutputStore` (feature `testing`) is an in-memory store for tests; clones share their outputs.
`check_output_store(&store).await` runs every rule against any store and panics naming the rule it broke, so a host
checks its own store the way Cicada checks its store backends. It uses container names of its own
(`cicada-output-check-a` and `-b`) and removes them before and after, so it runs on a store that holds other data.
It does not check atomicity after a crash, the guard under parallel writers, or a file at `/a` next to one at `/a/b`
(`copy_out` gives only regular files).
**Caveats.** Run it in a test with a Tokio runtime (`#[tokio::test]`); it is `async`.

## 5. Errors

**Where.** The new section of item 1.
**Text.** `OutputError { kind, message }`: `Invalid` (a name or path outside the rules), `Conflict` (a stale `after`),
`Unavailable` (the store could not be reached; the only kind `is_retryable` calls retryable), `Other`.
**Caveats.** None.

## Blocking

None.

## Status

Ran: `cargo test -p cicada-containers --all-features` (182 unit, 45 integration, 12 doctests passed),
`cargo clippy -p cicada-containers --all-targets` with and without `--all-features` (`-D warnings`, clean),
`cargo check -p cicada-containers` without features, `cargo test -p cicada-examples --test docs`. All prose above is
proposed text; none of it is written. The block of item 1 runs as a doctest of `cicada-containers`, not yet as a
guide doctest.
