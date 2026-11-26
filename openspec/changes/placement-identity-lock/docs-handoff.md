# Docs handoff for placement-identity-lock

Version: 0.25.0 (unreleased; opens the 0.25.0 batch). Refresh the dependency pins of `docs/how-to/install.md` once it
is published.

## What changed

A placement member notices a second machine under its node id (ADR 0071), a node can set the host name it writes to
its row, and `NodeIdentity` locks its folder.

- `cicada`: `NodeConfig::hostname: Option<String>`, default `None`: the host name the node writes to its row in the node
  table; `None` keeps the machine's own, looked up once when the node is built (`ActorSystem::builder(..).build()`).
  A set value is checked like `label` (not blank, no NUL, at most 200 characters; else initialization and start fail
  with `hostname must not be blank`, `hostname must not contain NUL` or `hostname must be at most 200 characters
  long`) and kept as given. `ActorSystem::hostname() -> Option<&str>` returns the value the node writes, known before
  the start. Proved by `a_configured_hostname_is_written_to_the_row_and_returned_by_the_node`
  (`crates/cicada/src/tests/runtime/operator.rs`) and `a_blank_a_nul_or_a_too_long_hostname_fails_start`
  (`crates/cicada/src/tests/runtime/startup.rs`). The command line has no flag for it.
- `cicada-actors`, the member: each round first reads its node's own row (`ActorSystem::node(id)`). A host name there
  that is set and differs from `ActorSystem::hostname()` makes the round skip the probe and the offer, send the pool
  `PlacementMsg::Duplicate { node, host }` (`node` the node's id, `host` the name found) and log a `tracing` error once
  for each other host name in a row: `placement member: another machine, '<host>', runs under this node id (<node>)
  on this host ('<own>'): no offers while it does; give each machine an identity folder of its own`. No host name of its
  own or in the row (such as a node not started, which has no row), or a failed read (a warning), checks nothing.
  Proved by `crates/cicada-actors/tests/placement_duplicate.rs`:
  `the_member_that_finds_another_host_skips_its_offers_logs_once_and_reports_the_duplicate` and
  `a_redeploy_under_a_new_host_name_reports_nothing`.
- `cicada-actors`, the pool: `Duplicate { node, host }` (`{"type": "Duplicate", "node": "mac-1", "host": "mac-b"}`),
  from anyone, both ids 1 through 200 characters, not blank, without NUL. For a live node it sets the conflict mark of
  ADR 0069 to `{instance, at, host}`: `instance` the node's own for a new mark (or the other instance of a mark an offer
  set), `at` the store time, `host` the reported name; a new mark sends one `conflict` notice. `PlacementConflict`
  gains the optional `host`, written only when set; the state's check accepts a mark naming the node's own instance only
  with a `host`. While the mark has a `host` the node gets no new places and keeps its holders (a drained node behaves
  alike). A mark set only by reports ends at the first turn of the pool one silence time after the last report; a mark
  set by offers and reports ends as an offer's mark does (the recorded instance offers one silence time after the
  later of the two); going silent ends every mark. A report of a silent node changes nothing; one of an unknown node or
  with an id out of bounds is refused for good. A conflicting offer on a reported node keeps the `host`. Proved by
  `a_reported_duplicate_alerts_once_gives_no_new_places_and_ends_one_silence_time_after_the_last_report`,
  `a_report_and_a_conflicting_offer_share_one_mark` and
  `a_report_of_a_silent_node_changes_nothing_and_of_an_unknown_node_is_refused` in
  `crates/cicada-actors/tests/placement.rs`; samples in `crates/cicada-actors/tests/wire_formats.rs`;
  `docs/reference/contracts/cicada.placement.json` regenerated.
- `cicada-actors`, the identity: `NodeIdentity::load_or_create(dir)` creates `dir` when missing and takes an exclusive
  OS lock on `dir/placement-identity.lock` (`File::try_lock`) before it reads or creates `placement-identity.json`. The
  identity and every clone hold the lock until the last is dropped or the process ends; `PlacementMember::new` keeps
  the identity, so the lock lasts as long as the member. A second load fails with `InvalidIdentity` whose new
  `kind()` is `IdentityErrorKind::Locked` (message: `placement identity <dir>/placement-identity.lock: locked by another
  process that runs this identity`); every other error is `IdentityErrorKind::Invalid` (`#[non_exhaustive]`). That
  holds in one process too: a second `load_or_create` of the folder while the first identity lives fails. On a file
  system without locks it logs a warning and goes on without one. Equality of `NodeIdentity` compares node id and
  instance. Proved by `a_second_load_fails_while_the_identity_lives_and_succeeds_after_all_are_dropped` in
  `crates/cicada-actors/tests/placement_member.rs` and the rustdoc example of `NodeIdentity::load_or_create`.

## 1. The duplicate check in the placement guide

**Where.** `docs/guide/actors.md`, `Placement pool`: the paragraph `**Two machines under one id.**` and its bullets,
and the member paragraph `**The member.**` with the bullets after its example.
**Text.** After the paragraph on offers of another instance, add: Two machines started from a copied identity folder
(a copied file, a cloned disk, a restored backup) share the node id and the instance, so their offers look like one
machine. They also write one node row in turn, so each member reads its node's row before every round: when its host
name differs from the one the node writes (`ActorSystem::hostname()`), the member skips that round's offer, logs an
error once and sends the pool `Duplicate { node, host }` (ADR 0071, linked as `../adr/0071-a-duplicate-node-reported-by-its-member.md` from the guide).
The pool marks the node `conflict: {instance, at, host}` and tells the alert target once with the kind `conflict`;
the node gets no new places while the mark has a host, and keeps its holders. The mark ends one silence time after the
last report; a node marked by both an offer and a report is cleared when both would be. Replace the bullet "A cloned
disk with the member's identity copied looks like one machine; the pool cannot tell." with: "A cloned disk with the
identity copied is found by the member only when the two machines have different host names; clones that keep one
host name are not told apart." Replace "The notice does not name the other instance; read the node's `conflict` in the
pool's state." with the same and "or host".
**Caveats.** A redeploy under a new host name does not trip the check, as the old node removes its row when it stops;
a crashed node's row lingers for at most one heartbeat of the new node (10 s). Set `NodeConfig::hostname` to give a
container a stable name; it changes nothing in placement besides this check.

## 2. The lock in the member section

**Where.** `docs/guide/actors.md`, `Placement pool`, the bullets after the member example (`Keep the identity folder
per machine.` and `A corrupt or unreadable identity file`).
**Text.** Add: `NodeIdentity` locks its folder (`placement-identity.lock`) while it or a clone lives; give it to the
member, which keeps it while it runs. A second process on the same folder fails to start with `InvalidIdentity`,
whose `kind()` is `IdentityErrorKind::Locked`; other identity errors are `IdentityErrorKind::Invalid`. Remove "Start one
process at a time on a missing file.": two processes starting at once can no longer both write the file.
**Caveats.** The lock covers one machine only. On one host with a shared volume, a rolling deploy's new container fails
with `Locked` until the old one stops. On a file system without locks it logs a warning and runs without one. Loading
the folder twice in one process fails too while the first identity lives (drop it first, such as in a test).

## 3. The pool's message and the mark's host

**Where.** `docs/guide/actors.md`, `Placement pool`: wherever the messages are listed (the operator recipes or the
`**Two machines under one id.**` paragraph) and where `conflict: {instance, at}` is named.
**Text.** `Duplicate { node, host }`, from anyone: a report of another machine under a live node's id; the mark gains
the optional `host`. An operator can send it by hand: `cicada send cicada.placement/runners --type Duplicate --payload
'{"node": "mac-1", "host": "mac-b"}'` (the same send route as the other recipes; not covered by a test of the CLI).
**Caveats.** A report of a silent node changes nothing; an unknown node is refused for good.

## 4. The host name setting

**Where.** `docs/guide/core.md`, `Configuration` table (row after `label`); `docs/operations.md`, `Node table`, the
paragraph `The host name is looked up once at start`.
**Text.** Table row: `hostname` | `None` | The host name written to the node table, such as a container's stable name;
`None` looks up the machine's own once when the node is built. Checked like `label`; read back with
`ActorSystem::hostname()`. Operations: "The host name is the machine's, looked up once when the node is built, or
`NodeConfig::hostname` when set (an invalid set value fails the start, unlike a lookup that gives no usable name)."
**Caveats.** No `cicada-app` flag or variable sets it; a host sets it in its `NodeConfig`.

## 5. Upgrading to 0.25.0

**Where.** `docs/how-to/upgrading.md`, a new `Upgrade to 0.25.0` above `Upgrade to 0.23.0`.
**Text.** **Version 0.25.0 changes the contract of `cicada.placement`.** `PlacementMsg` gains `Duplicate` and the
conflict mark the optional `host`, so the contract hash changes. The type name is fixed, so hosts with pools use a
maintenance window: stop every host of the type, delete its pools and then its registration, and start the new hosts,
which create the pools again. Rust code: add `hostname: None` to `NodeConfig` literals without `..NodeConfig::default()`,
`host: None` to `PlacementConflict` literals, and a `Duplicate` arm to exhaustive matches of `PlacementMsg`; drop a
`NodeIdentity` before loading its folder again.
**Caveats.** A process that shares an identity folder with another on the same machine fails with `Locked` until the
other stops.

## 6. Architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row, the placement part (`conflict` once when another instance
offers …, ADR 0069) and `PlacementMember … under a NodeIdentity kept in placement-identity.json across restarts`.
**Text.** Add after the ADR 0069 clause: "or when its member reports another host name in the node's row (`Duplicate`,
ADR 0071), which marks the node with that host and gives it no new places until one silence time after the last
report". Add to the member: "which locks its folder while it lives and, each round, compares its node row's host name
with the one its node writes (`NodeConfig::hostname`, `ActorSystem::hostname()`)".

## Blocking

None.

## Status

Ran: `cargo test -p cicada` (all passed, PostgreSQL tests included), `cargo test -p cicada-actors` (all passed),
`cargo test -p cicada-examples --test docs` and `--doc` (see the review request's report). All prose above is
proposed; nothing was prepared by the owner. The guide's member example (`docs/guide/actors.md`, doctest `@@23`) loads
the identity once and builds `NodeConfig` with `..NodeConfig::default()`, so it compiles unchanged.
