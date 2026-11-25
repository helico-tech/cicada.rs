# Proposal

## Why

A host follows a running container's log and keeps it outside the container, so it survives the container's removal
and can be served to users. Factory does this in its runner; this line moves it into `cicada-containers` (request from the Factory session, 2026-11-25; the owner accepted the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- An archiver that follows the log of each container of a container job with `logs_since` and appends the lines and
  the cursor to the `OutputStore` in one write, resuming from the stored cursor after a restart. It runs outside actor
  turns (a page is too big for actor state), for instance as an `App::task` or a task the host spawns.
- It stops at `Unsupported` (`container-logs-unreadable`), retries `DaemonUnavailable` and `Other` gently, and drains
  the rest once the container ended.
- A read-back helper (from an offset, with a limit) for a host's HTTP route or API.

To decide in the design: how the archiver learns which containers to follow (the actor's state, a list by label, or a
message from the actor), its poll interval, and how it hands over to the end of the container (`container-output-collect`).

## Out of scope

Progress files, output folders, retention.

## Order

After `container-output-store`, before `container-progress`.

## Impact

`crates/cicada-containers`, perhaps `cicada-app` if it runs as an app task. Additive.
