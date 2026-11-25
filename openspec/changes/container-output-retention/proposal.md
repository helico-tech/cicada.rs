# Proposal

## Why

A host keeps a container's stored logs and files for a time after its end, then deletes them. Factory does this
itself (request from the Factory session, 2026-11-25; the owner accepted the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- After a retention time from a container's end, its logs and files are deleted from the `OutputStore`, safely
  repeated, and the deletion survives restarts.

To decide in the design: who schedules it (the container actor with its own timer, or the scheduler actor), the
setting, and what happens to the container actor's own record.

From the design review of `container-output-store`: after `remove` a container's cursor is `None` again, so retention
removes only once no archiver runs for it; and whether outputs whose actor is gone stay findable (a list of stored
containers) is decided here.

## Out of scope

Retention of the actor's state (Cicada's retention covers messages).

## Order

After `container-output-collect`, before `container-orphan-alert`.

## Impact

`crates/cicada-containers`. Additive.
