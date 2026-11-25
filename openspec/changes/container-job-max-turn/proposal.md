# Proposal

## Why

`ContainerActor` pulls a missing image inside one turn, so a runner node raises `NodeConfig::default_max_turn` to
30 minutes. `CICADA_MAX_TURN` / `--max-turn` are applied after the host's config and can lower it again, and a slow
pull is then cut off at every attempt (Factory issue 0020, request from the Factory session, 2026-11-25).
`Actor::MAX_TURN` already wins over the node's default per type, but a host cannot set it for `ContainerActor<J>`.

## What Changes

- `ContainerJob` gets `const MAX_TURN: Option<Duration> = None`, and `ContainerActor<J>` uses it as its
  `Actor::MAX_TURN`. Factory would set 35 minutes and drop its raised default.

To decide in the design: whether the actor adds anything to the job's value; the rustdoc on what one turn of the
container actor holds (a pull, a create, copies in).

## Out of scope

Moving the pull out of the turn. The order of the node settings.

## Order

After `watch-absent-target`, before `container-logs-unreadable`.

## Impact

`crates/cicada-containers` (`ContainerJob`, the actor). Additive (a constant with a default): a patch.
