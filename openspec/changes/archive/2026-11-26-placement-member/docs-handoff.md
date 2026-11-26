# Docs handoff for placement-member

Version: 0.24.0 (unreleased; the placement line ships as one minor version). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

A member that offers a node's places to a placement pool while the node can run work, under an identity kept across
restarts. All in `cicada-actors`, exported at the crate root.

- `NodeIdentity::load_or_create(dir) -> Result<NodeIdentity, InvalidIdentity>`: reads `dir/placement-identity.json`,
  `{"node": "node-3f9c0a1b2c3d4e5f", "instance": "<32 hex>"}`; when the file is missing it creates `dir` (with its
  parents) and the file, with the node id `node-<16 hex digits>` and a random instance, written to a temporary file in
  `dir` and renamed into place. A file that cannot be read, is not that JSON (unknown fields included), or holds an id
  out of the offer's bounds (1 through 200 characters, not blank, without NUL) is an `InvalidIdentity`, which prints
  the path and why; the file stays as it is and no new identity is made. `node_id() -> NodeId` (the default for
  `NodeConfig::node_id`) and `instance() -> &str`.
- `MemberSettings::new()` (interval 15 s), `with_interval(d) -> Result<Self, InvalidPlacement>` (zero refused, the
  error prints `a member's interval is zero`), `interval()`.
- `Capacity { labels: Vec<String>, places: u32 }` and `Capacity::new(labels, places)`.
- `PlacementMember::new(pool, identity, settings, probe)`, where `probe` is an `FnMut() -> impl Future<Output =
  Option<Capacity>>`, and `async fn run(self, system: ActorSystem, stop: impl Future<Output = ()>)`. A round runs at
  the start of `run` and then every interval (a round that takes longer delays the next). `Some(capacity)` sends
  `Offer { node: system.node_id(), instance: identity.instance(), labels, places }`; `None` sends nothing. An offer out
  of bounds (`PlacementMsg::offer` fails) and a send that fails or is not accepted (such as `NoSuchInstance` for a pool
  not created yet) are logged as `tracing` warnings and tried next round. Nothing ends `run` but `stop`, which also
  ends it while a probe or send hangs.
- The rustdoc of `PlacementMsg::Offer::instance`, and so `docs/reference/contracts/cicada.placement.json`, says the
  instance is kept across restarts.

Proved by `crates/cicada-actors/tests/placement_member.rs`: `the_identity_is_made_once_and_read_the_same_after_a_restart`,
`an_invalid_identity_file_is_refused_and_left_as_it_is`, `an_unreadable_identity_file_is_refused`,
`a_zero_interval_is_refused`, `the_member_offers_while_the_probe_answers_and_the_node_goes_silent_after` (also the
node's own id under `--node-id`), `a_failed_send_is_tried_again_in_the_next_round` and
`the_stop_ends_the_member_also_while_the_probe_hangs`; the rustdoc examples of `NodeIdentity::load_or_create`,
`MemberSettings` and `PlacementMember`.

## 1. Running a member

**Where.** `docs/guide/actors.md`, `Placement pool`: step 3 of the numbered list, and a new paragraph after
`**Offers.**`, such as `**The member.**`.
**Text.** Step 3 becomes: "On each member node, run a `PlacementMember` as an app task; it sends
`PlacementMsg::offer(node, instance, labels, places)` every 15 s while its probe answers (keep the interval well below
the silence)." The new paragraph: Each node keeps its identity in a folder of its own with
`NodeIdentity::load_or_create(dir)`: the first start writes `placement-identity.json` with a node id and an instance;
later starts read the same, so a restart is the same machine to the pool. Use `node_id()` as `NodeConfig::node_id`; a
`--node-id` flag or `CICADA_NODE_ID` still wins, and the member always offers the node's own id
(`ActorSystem::node_id`), the one its claims and pins use. The probe says what the node can take now: return
`Some(Capacity::new(labels, places))` when the runtime answers and `None` when it does not, and the node goes silent at
the pool by itself. A failed send is logged and tried at the next round. Suggested example (no_run, as it needs Docker
and a store), with `cicada_containers::DockerRuntime` and its `info()` as the probe:

```rust
let id = NodeIdentity::load_or_create("/var/lib/factory/placement")?;
let config = NodeConfig { node_id: id.node_id(), ..NodeConfig::default() };
let runtime = Arc::new(DockerRuntime::connect()?);
let member = PlacementMember::new(Placement::named("runners"), id, MemberSettings::new(), move || {
    let runtime = runtime.clone();
    async move {
        let info = runtime.info().await.ok()?; // no answer: no offer this round
        Some(Capacity::new([info.arch.as_str(), "docker"], 4))
    }
});
App::new("factory")
    .node_config(config)
    .task("placement", move |system, stopping| async move {
        member.run(system, stopping.wait()).await;
        Ok(())
    })
```

`DockerRuntime::connect`, `RuntimeInfo::arch` and `App::node_config` exist as named; the block is not compiled here.
The rustdoc example of `PlacementMember` (in `crates/cicada-actors/src/placement/member.rs`) is a compiled variant on
`TestActors`.
**Caveats.**
- Keep the identity folder per machine. A copied file, or a cloned disk that holds it, makes two machines one to the
  pool, which then cannot report the conflict.
- A corrupt or unreadable identity file stops the start with `InvalidIdentity`; delete the file to take a new identity
  on purpose (the pool then sees a new instance: a conflict until the old entry goes silent).
- One process creates the file: two processes that start at once on a missing file may each write one.
- The member cannot read the pool's silence: the host keeps the interval below it.
- A probe that hangs holds back the rounds; the host bounds it with its own timeout when it must.

## 2. Offers: the instance is kept

**Where.** `docs/guide/actors.md`, `Placement pool`, the paragraph `**Offers.**`, and the comment in the example
`// A member offers; `instance` is drawn at each process start.`
**Text.** "`instance` an id the member draws at each process start" becomes "`instance` an id kept across restarts
(`NodeIdentity::instance`), so two machines offering under one node id can be told apart". The example comment becomes
`// A member offers; `instance` is its NodeIdentity's, kept across restarts.` If the conflicts prose (handoff of
`placement-conflicts`, caveat on instances drawn at each start) is in the guide, its restart caveat now applies only to
members that do not use `NodeIdentity`.

## 3. The ready-made actors list in the architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crates table, the placement pool entry.
**Text.** Add: `PlacementMember`, a loop outside turns run as an app task, offers the node's places while the host's
probe answers, under a `NodeIdentity` kept in `placement-identity.json` across restarts.

## Blocking

None.

## Status

Proposed text only; nothing protected was edited. Checks: see the report of this build.
