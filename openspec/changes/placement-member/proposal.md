# Proposal

## Why

Each node must offer its places only while its runtime answers, under an id that stays the same across restarts and
that another machine cannot take. Factory does this with a machine-id file and its own task (request from the Factory session, 2026-11-25; the owner accepted the split (CO-SPLIT, PL-SPLIT, AB-ORDER: container outputs first)).

## What Changes

- A member that offers a node's places at an interval while a check answers (for containers: `info()` of the
  runtime), as an `App::task` (`app-tasks`), with a stable node id across restarts that is refused when another
  machine claims it; Cicada's node records may give part of it.

To decide in the design: where it lives (`cicada-actors`, `cicada-app` or `cicada-containers` for the container check),
the id's source, the claim check.

## Out of scope

Supervising the node.

## Order

After `placement-drain-and-gone`, before `placement-operator-commands`.

## Impact

`crates/cicada-actors` and perhaps `cicada-app` / `cicada-containers`. Additive.
