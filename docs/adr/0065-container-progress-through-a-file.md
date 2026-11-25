# 0065. Container progress through a file

Date: 2026-11-25

## Status

Accepted for container-progress. Builds on ADR 0057 and ADR 0062.

## Context

A job's program reports progress while it runs (Factory: done and total of a test run). Hosts read a file in the
container at an interval and tell the job's owner. The container actor polls each running container already, and only
a turn can send a message.

## Decision

- `ContainerMsg::Start` gets an optional `progressType`, kept in `EndTarget`. With it, the actor sets
  `CICADA_PROGRESS_FILE` to the job's `PROGRESS_FILE` in the container's environment, after the job's `spec`, reads that
  file with `copy_out` (at most 64 KiB, one file) at each
  running poll and once before the end is delivered, and sends `ContainerProgress { key, seq, text }` to the end's
  target as `progressType` when the text changed, `seq` counting from 1.
- Progress is best effort: a missing, oversized or unreadable file sends nothing and is not a runtime error; a refused
  send does not block the end.
- The progress path is checked at the create; a refused path ends the container `notStarted`. NUL becomes U+FFFD.
- The last progress is sent in the end's commit, before the end; a retried progress can arrive after it, so owners
  drop a progress whose `seq` is not above the last they handled.
- The actor's state keeps the last `seq` and a SHA-256 of the last text. With the new field of `Start` and
  `EndTarget`, the contract of every container job type changes: a breaking release.

## Consequences

An owner gets ordered progress and then the end, without a reader of its own. A program must write the file
atomically (a temp name and a rename). Hosts on 0.22.x follow "Changing a contract" for their container job types.
