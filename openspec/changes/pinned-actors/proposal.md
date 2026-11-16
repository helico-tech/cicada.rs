# Proposal

## Why

Any node that hosts a type may claim any instance of it. Some actors must run on one machine: an actor that manages
a Docker container, or a process, can only reach it on the machine where it runs, for example to restart it or to
run a command in it. Today the only way is to host the type on that one node, which fails as soon as each machine
needs its own instance: a type's name is a compile-time constant (`Actor::TYPE`), so a host cannot register "the
monitor of host A" and "the monitor of host B" as two types.

When an actor only needs to *observe* the machine, pinning is not needed: a local task on the machine sends messages
(for example `Beat` to a `heartbeat-watch`) to an actor that runs anywhere. Pinning is for turns that must *act* on
the machine.

## What Changes

- **A pin at create.** Creating an actor may name a node id. Only the node with that id may claim the actor's turns.
  An actor with no pin behaves as today.
- **Messages wait for the node.** While the pinned node is down, or does not host the type, the actor's messages stay
  pending. Nothing fails because of the pin. The pin shows in the actor's snapshot and the mailbox listing, so an
  operator can see why messages wait.
- **The claim respects the pin**, in both stores, checked by a new case in the conformance suite.
- Sends to a pinned actor work from any node, as today.
- **Keep the placement primitive small.** The child-process example moves to `pinned-process-example`, immediately
  after this change, under the owner's autonomous delegation on 2026-11-16. It needs its own outside-effect and
  host-restart contract; splitting it preserves the original scope without hiding it inside store/API work.

Decisions made under the owner's delegation (2026-11-16; Opus 5.5 reviews):

- Store an optional immutable node/worker id on the actor row, separate from its address.
- Apply the same claim-owner comparison to direct nodes and remote workers; node ids are trusted routing identities,
  not authenticated machine identities. Require a stable unique configured id where placement must survive restart.
- Keep existing create calls unpinned; add options variants, including typed/raw child creates and remote completion.
- Keep existing instances untouched on repeat creation. New conflicting creates in one turn are invalid.
- Show placement in actor snapshots/listings and CLI describe; use existing pagination to inspect pending placement.
  Do not add node-detail fanout, new list filters or pin mutation.
- Drain all old direct-store runtimes before migration/activation: old claim code cannot enforce pins.

## Out of scope

The child-process example and its recovery adapter (`pinned-process-example`). Moving a pin (delete and create again). Pinning by label or selector. Failing messages after a timeout when the
pinned node stays down: a watcher can use `node-down-alert` instead. Pinning a whole type to a node.

A live connection that can arrive at any node, such as a websocket. A pin fits it badly: a reconnect can land on
another node, and a pin does not move. Its way needs no pin and works today: the node that holds the socket follows
the session actor's state through the change hints and pushes; inbound frames are messages. That way, and the one of
the worked example, are compared in `openspec/changes/actor-design-skill/research/cicada-fit.md`; teaching both is
part of `actor-design-guide`; the executable process example follows in `pinned-process-example`.

## Order

After `actor-design-review`. Before `pinned-process-example`.

## Impact

A change to the claim rule, so a new ADR and its `docs/architecture.md` text in the docs handoff. `crates/cicada`
(create, the claim in both stores, an appended migration, the conformance suite, the fuzz and runtime tests), the
create body in `api` and `docs/openapi.json`, `cicada-client`, `cicada-app` (create and describe commands,
`docs/reference/cli.md`), and handoffs for the core guide, the HTTP guide and `docs/operations.md`. A pinned actor
whose node never returns keeps its messages pending; the design names how an operator finds and removes it.
