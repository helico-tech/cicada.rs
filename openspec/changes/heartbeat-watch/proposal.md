# Proposal

## Why

A host often needs to know that something stopped happening: a nightly import that did not run, a device that went
silent, a partner that stopped sending files. Watches report an actor that failed or stopped, but not an outside
thing that is simply quiet. Hosts write this by hand with a delayed self-send that each sign of life replaces.

The debouncer is close but not the same. It also fires after a quiet period, but its longest wait fires while
submissions still arrive, which here is a false alarm, and each submission must carry the alarm itself. A heartbeat
watch has no longest wait, keeps the alarm in its settings, and raises it once per silence.

It is the first small business-level actor after the three integration actors, and it gives a webhook sink something
useful to send.

## What Changes

- **A heartbeat watch actor** (`cicada.heartbeat-watch`), one instance per watched thing. Its state holds the
  allowed silence, a target with a stored alarm message, the armed timer and a status: `alive` or `silent`.
- **A public `Beat` message** is the sign of life. It replaces the armed timer with one that is due after the
  allowed silence, in one commit, and sets the status to `alive`.
- **When the timer is due** with no `Beat` since, the actor sends the alarm to the target once and sets the status to
  `silent`. It arms nothing more: one silence is one alarm, until the next `Beat` starts a new round.
- **Starting.** The watch counts from its first `Beat`. The design decides whether a public `Start` is also needed,
  for a thing that may never send its first sign of life.
- **Recovery.** The design decides whether the first `Beat` after a silence sends a second stored message ("it is
  back"), or whether the status in the state is enough for the first version.
- As with the other actors, the timer message must be self-sent and armed, and the due time is a lower bound: a
  paused or slow watch raises its alarm late, never early.

## Out of scope

Repeating the alarm while the silence lasts, and escalating to a second target. Checking the outside thing actively:
that is the HTTP poller. Watching a Cicada actor for failure: watches do that. Expecting a beat at fixed clock times
("every day before 06:00") instead of within a period. Changing the settings of an existing watch: delete and
recreate it, as with the debouncer.

## Order

After `webhook-inbox`. No following change.

## Impact

A new actor in `cicada-actors` with its contract in `docs/reference/contracts/`, tests on virtual time, a worked
example, a route in the consumer skill, and a handoff to the docs session for the actors guide. No change to the
core, the store, a guarantee or an existing wire format.
