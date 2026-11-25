# Proposal

## Why

A job's container writes its progress to a file, and the host reads it at an interval and once more at the end and
tells the job's owner. Factory does this in its runner (request from the Factory session, 2026-11-25; the owner accepted
the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- A progress file in the container, its path given to the program in an environment variable, read with `copy_out`
  at an interval while the container runs and once more at its end, and delivered to the owner as messages with a
  sequence number, only when it changed.

To decide in the design: who reads (the container actor in its polls, or the archiver of `container-log-archive`), the
message and its contract, the size limit, the env variable's name, what a missing or half-written file gives.

## Out of scope

Parsing the file: its content goes to the owner as it is.

## Order

After `container-log-archive`, before `container-output-collect`.

## Impact

`crates/cicada-containers` (the actor or the archiver, a message of the end protocol). Additive or a contract change:
the design says which.
