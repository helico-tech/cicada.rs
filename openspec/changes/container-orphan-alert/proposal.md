# Proposal

## Why

Containers with a job's name prefix that no container actor owns (left from a lost store, a test, a crash between a
create and its commit) run on unnoticed. Factory alerts on them (request from the Factory session, 2026-11-25; the owner accepted the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- A check that lists the containers of a job's label or name prefix and alerts on those whose actor does not exist,
  once per container, without removing them.

To decide in the design: where the alert goes (a log line, a message to a host's actor, an observer), how often, and
the line between this and `sweep`.

## Out of scope

Removing orphans automatically.

## Order

After `container-output-retention`, before `placement-offers`.

## Impact

`crates/cicada-containers`. Additive.
