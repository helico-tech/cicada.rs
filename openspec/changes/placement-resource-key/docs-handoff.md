# Docs handoff for placement-resource-key

Version: 0.25.0 (unreleased, the batch that placement-identity-lock opened). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

Two runner nodes on one machine, each with its own identity, both offered one Docker daemon, so its places counted
twice. An offer can now name a resource key, and the pool lets one live node hold a key (ADR 0072).

- `cicada-containers`: `RuntimeInfo` (`#[non_exhaustive]`) gains `id: Option<String>`, the id of the runtime instance,
  and `RuntimeInfo::with_id(id)`. `DockerRuntime::info()` fills it from the `ID` of Docker's `GET /info` (empty or
  missing: `None`), the same for every node that talks to that daemon. That is a second call next to `/version`; an
  error of either fails `info()` as before. `/info` gathers the daemon's whole state and can take seconds on a busy
  daemon, and a probe calls it every round. Docker does not promise the id's format, only that it names the daemon.
  `FakeRuntime` reports no id until `set_info(RuntimeInfo::new(..).with_id(..))` sets one. Proved by
  `runtime_info_takes_the_daemon_id_of_info_and_an_empty_one_is_none` (`docker/map.rs`),
  `info_answers_amd64_on_linux_until_a_test_sets_another_machine` (`fake.rs`) and the E2E step
  `info_gives_linux_and_the_arch_the_cli_prints`, which compares the id with `docker info --format '{{.ID}}'`.
- `cicada-actors`, the member: `Capacity` gains the public field `resource: Option<String>` and the builder
  `Capacity::resource(key)`. The member sends it as `Offer`'s new optional `resource` (1 through 200 characters without
  NUL, blank allowed); a key out of bounds makes the member send no offer that round and log a warning. Proved by
  `a_capacity_with_a_resource_key_offers_it` (`tests/placement_member.rs`).
- `cicada-actors`, the pool: `Offer { node, instance, labels, places, resource }`
  (`{"type": "Offer", …, "resource": "4f2a:ZQ7Y"}`, `resource` written only when set; out of bounds refused for good).
  The node entry records the key of its last offer (`PlacementNode::resource`, written only when set). One live node
  holds a key: an offer whose key a different live node holds changes nothing for the offering node. A new node is not
  added; a known one keeps its last entry, so it goes silent if it keeps offering only that key, and a silent one is not
  brought back (no `back`). The pool lists the contest in the state's new `contested`
  (`[{"resource": "4f2a:ZQ7Y", "node": "mac-2", "at": "…"}]`, `PlacementContest`), one entry per key and offering node,
  `at` the store time of the last such offer, and sends one `conflict` notice naming the offering node when an entry is
  added (the notice does not name the key; read `contested`). The check removes an entry one silence time after its
  `at`, so a later contest is told again; the check now runs while a contest is listed, also when no node is live
  (`armed` is set exactly while a node is live or a contest is listed). The list holds at most `maxNodes` entries; a
  new contest past that is refused for good. When the holder goes silent or is declared gone, the next offer of the key
  takes it; the runs on the old holder may still use the resource for a while, a brief overbooking. Offers without a
  key are not checked, and two different keys for one resource are not told apart. The node's conflict mark of ADR 0069
  and ADR 0071 is unchanged. A decoded state with two live holders of one key is refused. Proved by
  `a_new_node_offering_a_held_key_is_not_added_and_alerted_once`,
  `a_known_node_offering_a_held_key_is_ignored_and_alerted_once_even_after_it_goes_silent`,
  `a_contest_ends_one_silence_time_after_its_last_offer_and_a_later_one_alerts_again`,
  `the_second_node_takes_the_key_after_the_holder_goes_silent`, `offers_without_a_key_are_unaffected` and
  `a_contest_past_max_nodes_is_refused` in `crates/cicada-actors/tests/placement.rs`; decode and wire tests in
  `src/placement.rs` and `tests/wire_formats.rs`; `docs/reference/contracts/cicada.placement.json` regenerated.
- Two fixes to placement-identity-lock's member code (from its code review), also in 0.25.0: a read-only identity
  folder that holds `placement-identity.json` now loads without a lock and a warning, whether the lock file is missing
  or there but read-only (before: `Invalid`); and a member that logged a duplicate host logs it again after a round
  that found no other host (before: never again for the same host). Proved by
  `a_read_only_folder_loads_the_identity_without_a_lock` (`tests/placement_member.rs`) and
  `a_second_duplicate_with_the_same_host_is_logged_again` (`tests/placement_duplicate.rs`).

## 1. The daemon's id in the containers guide

**Where.** `docs/guide/containers.md`, `The runtime's machine`.
**Text.** After the first paragraph: `RuntimeInfo` also has `id`, Docker's daemon id from `/info`, the same for every
node that talks to that daemon, or `None` when the runtime gives none. Use it as a placement member's resource key
(link to the actors guide's member section) so two nodes on one daemon do not offer its places twice. Fetching it costs
a second call, to `/info`, which can take seconds on a busy daemon. In the tests, `FakeRuntime::set_info` with
`RuntimeInfo::new("arm64", "linux").with_id("daemon-1")` sets one; by default the fake has none.
**Caveats.** Docker does not promise the id's format: compare it, do not parse it.

## 2. The resource key in the member section

**Where.** `docs/guide/actors.md`, `Placement pool`, the paragraph `**The member.**` and its example.
**Text.** The probe can name the resource its places belong to with `Capacity::resource(key)`; with Docker, the
daemon's id. Proposed change of the example's probe body (the guide's `@@23` doctest; it compiles against this branch's
API, as `a_capacity_with_a_resource_key_offers_it` shows the builder):

```rust
let info = runtime.info().await.ok()?; // no answer: no offer this round
let capacity = Capacity::new([info.arch.as_str(), "docker"], 4);
Some(match info.id {
    Some(id) => capacity.resource(id), // one daemon counts once, however many nodes share it
    None => capacity,
})
```

A bullet after the example: a key is 1 through 200 characters without NUL; out of bounds, the member sends no offer
and warns each round.
**Caveats.** Each node on a shared daemon still needs its own identity folder; the key does not replace it.

## 3. The rule and its alert in the pool section

**Where.** `docs/guide/actors.md`, `Placement pool`: a new paragraph after `**Two machines under one id.**` (such as
`**One resource, two nodes.**`), the `**Alerts.**` paragraph, and the operator paragraph that lists the state's fields
(`Read the pool with cicada describe actor …`).
**Text.** One live node holds a resource key: the first live node whose offer recorded it. A second live node that
offers the same key is ignored (a new node is not added; a known one keeps its last entry and goes silent if it keeps
offering only that key) and is listed in the state's `contested` with the key and the time of its last such offer; the
alert target gets one `conflict` notice naming the ignored node. The entry goes one silence time after that node's
last such offer, and a later contest is told again. When the holder goes silent or is declared gone, the next offer of
the key takes it. In **Alerts**: `conflict` is also sent once per contest; read `contested` for the key. In the state's
fields: `nodes` also show `resource`, and the state shows `contested` (`resource`, `node`, `at`).
**Caveats.** After a holder goes silent its runs may still use the resource while the new holder gets new places: a
brief overbooking. Offers without a key are not checked, and two keys for one resource are not told apart. A pool lists
at most `maxNodes` contests; a new one past that is refused (a failed `Offer`).

## 4. Upgrading

**Where.** `docs/how-to/upgrading.md`, the 0.25.0 section that placement-identity-lock's handoff asked for.
**Text.** The contract change of `cicada.placement` in 0.25.0 also covers the offer's and the node's `resource` and the
state's `contested`: the same maintenance-window procedure. Rust code that builds or matches `PlacementMsg::Offer`,
`PlacementNode`, `PlacementState` or `Capacity` without `..` gains the new fields (`resource: None`,
`contested: vec![]`); prefer `Capacity::new`.
**Caveats.** None beyond the existing procedure.

## 5. The identity fixes

**Where.** `docs/guide/actors.md`, `Placement pool`, the bullets after the member example (the lock text that
placement-identity-lock's handoff proposed).
**Text.** Where the lock's limits are listed: a read-only folder that holds the identity file, like a file system
without locks, loads without a lock and logs a warning. Where the duplicate log is described: it is logged once per run
of a duplicate host; after a round that finds no other host, a later one is logged again.
**Caveats.** Without a lock, two processes on one read-only folder are not stopped.

## Blocking

None.

## Status

Ran `cargo test -p cicada-examples --doc` and `--test docs` on this branch: they pass (see the build report). All the
text above is proposed; the docs session writes it. No protected file was edited.
