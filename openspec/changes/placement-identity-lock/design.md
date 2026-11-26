# Design

A copied identity folder (a copied disk, a restored backup, a cloned folder) gives two machines the same node id and
instance; the pool then sees one node, and runs placed there may start on either machine. Factory's owner decided
Factory builds no guard of its own (epic 0008; blocks its merge).

## Decisions

1. **Two machines under one id are seen in the node table, at run time.** Two nodes with one id share one node row,
   upserted by id at each heartbeat (`ON CONFLICT (id) … hostname = excluded.hostname`), so its host name flips between
   them while both run. Each round, first (before the probe), the member reads its own row (`ActorSystem::node(id)`, one
   row); a host name that is set and differs from the one this node writes (`ActorSystem::hostname()`, design review
   round 2) means another machine wrote it since the last heartbeat. The member then sends no offer that round, logs an
   error once per
   duplicate, and sends the pool `Duplicate { node, host }`. A redeploy (a new container host name, one machine at a
   time) never trips it, and the overlap of an old and a new replica clears by itself. Not taken: a host name recorded
   in
   the identity file (design review: a container's host name changes at every redeploy, so a kept identity would refuse
   to load).
2. **The pool marks the duplicate and alerts once; ADR 0071.** `Duplicate { node, host }` (from anyone; it is a
   report) sets the node's conflict mark of ADR 0069 with the other machine's `host` (the mark gains an optional
   `host`; the decode accepts a mark naming the node's own instance only when `host` is set) and sends one `conflict`
   alert per mark; the node gets no new places while marked. A mark with `host` ends one silence time after the last
   report; ADR 0069's rules still end a mark without `host`; a mark set both ways ends when both would. Unknown nodes
   are
   refused for good. ADR 0071 extends ADR 0069 (which says a cloned disk looks like one machine) without editing it.
3. **One process per identity folder: an OS file lock, kept with the member.** `load_or_create` first takes an exclusive
   lock on `dir/placement-identity.lock` (`File::try_lock`, Rust 1.89), then reads or creates the file, so two
   processes starting at once cannot both create it. The lock lives in an `Arc` inside `NodeIdentity`, which stays
   `Clone` (equality ignores the lock), and `PlacementMember` keeps the whole identity, so the lock lives as long as the
   member's task (design review, blocking 1). A second process fails with `Locked`; a file system that does not support
   locks (`try_lock` errors other than "would block") logs a warning and goes on without a lock.
4. **Errors as a kind.** `InvalidIdentity` keeps its message and gains `kind()` returning a `#[non_exhaustive]`
   `IdentityErrorKind::{Invalid, Locked}`; existing code keeps compiling.
5. **Host names: what the node writes, and an override.** `NodeConfig` gains `hostname: Option<String>`: the host name
   the node writes to its row, `None` for the machine's own (today's behaviour, read once at start through the private
   `usable_hostname`). `ActorSystem::hostname()` returns the value the node writes, which the member compares; a
   machine renamed while running keeps writing the old value, so it is no false duplicate. The override also gives
   containers a stable name and lets tests run two systems under one id with different names. Without a host name, here
   or in the row, the round skips the check. `NodeConfig` is not `#[non_exhaustive]`, so a struct literal without
   `..NodeConfig::default()` needs the field: the 0.25.0 upgrade notes say so.
6. **Inside the 0.25.0 line.** A pool message, an optional field of the conflict mark (a contract change, released with
   the line), `NodeConfig::hostname` and `ActorSystem::hostname()`, the error kind; the identity file format does not
   change. Limits, in the rustdoc: a copied disk that keeps its host name is not detected (the lock stops a second
   process on one machine only); on one host with a shared volume, a rolling deploy's new container fails with
   `Locked` until the old one stops.

## Tests

On virtual time with `TestActors`: two systems under one node id with different `NodeConfig::hostname` on one store
make the member skip its offer, log once and send `Duplicate`; the pool alerts once, gives no new places while marked,
and clears the mark one silence time
after the last report; no host name skips the check; a redeploy's new host name with the old row gone does not trip
it. In a temp folder: a second `load_or_create` while the first identity (or a clone of it) lives fails with `Locked`
and succeeds after all are dropped; existing tests that load twice drop the first identity first.

## Docs

The handoff adds the duplicate check, the lock and its limits (clones with the same host name, file systems without
locks) to the guide's member section, and the `Duplicate` message and the mark's `host` to the pool section.
