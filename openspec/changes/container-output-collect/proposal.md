# Proposal

## Why

At a container's end, a host drains the rest of its log, copies an output folder out within limits into its store
and tells the owner what was stored and what was skipped. Factory does this one container after another in its
runner, so a slow copy holds up the others (Factory issue 0005; request from the Factory session, 2026-11-25; the owner
accepted the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- At the end of a container: the rest of the log into the `OutputStore`, `copy_out` of the job's output folder
  within limits into the store, and a report to the owner with the stored files and the skipped ones with their
  reasons (a count and the first 10).
- Each container's collection runs on its own, so a slow copy does not hold up other containers.

To decide in the design: where it runs (outside turns, as the copy is too big for state), how the end waits for it, the
report's contract, the limits' defaults, what a failed copy gives.

## Out of scope

Retention, orphans.

## Order

After `container-progress`, before `container-output-retention`.

## Impact

`crates/cicada-containers`. A contract change if the end report changes: the design says which.
