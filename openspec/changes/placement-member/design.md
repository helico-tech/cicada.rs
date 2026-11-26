# Design

The fifth change of the placement-pool line. Each node offers its places to a pool only while it can run work, under
a node id that stays the same across restarts and that another machine cannot take unnoticed. Factory does this with
a machine-id file and its own task.

```rust
let id = NodeIdentity::load_or_create("/var/lib/factory/placement")?;     // node id + instance, kept in one file
let config = NodeConfig { node_id: id.node_id(), ..NodeConfig::default() };  // the default; --node-id still wins
let member = PlacementMember::new(Placement::named("runners"), id, MemberSettings::new(), move || async move {
    let info = runtime.info().await.ok()?;                                     // no answer: no offer this round
    Some(Capacity::new([info.arch.as_str(), "docker"], 4))
});
App::new("factory").node_config(config)
    .task("placement", move |system, stopping| async move { member.run(system, stopping.wait()).await; Ok(()) });
```

## Decisions

1. **A loop outside turns, like the archiver.** `PlacementMember::new(pool, identity, settings, probe)` and
   `async fn run(self, system: ActorSystem, stop: impl Future<Output = ()>)` in `cicada-actors` (which has `tokio`);
   a host runs it as an `App::task`. Every `interval` (default 15 s; zero refused by `MemberSettings`; the host keeps
   it below the pool's silence, which the member cannot read) it calls `probe()`; `Some(Capacity { labels, places })`
   sends `Offer { node: system.node_id(), instance, labels, places }` to the pool, `None` sends nothing, so a node whose
   runtime does not answer goes silent at the pool by itself. A failed send is logged and tried next round. `run` ends
   on `stop` also while a probe hangs (the probe is raced against `stop`). Not taken: in `cicada-app` (the pool is a
   ready-made actor; the loop needs only `ActorSystem`); in `cicada-containers` (the probe is the host's).
2. **The offered node id is the node's own.** The member offers `system.node_id()`, the id the node's claims and pins
   use, so a `--node-id` flag or `CICADA_NODE_ID` that overrides `NodeConfig::node_id` cannot split them (design
   review, blocking 2).
3. **A stable identity in one file, never silently replaced.** `NodeIdentity::load_or_create(dir)` creates `dir` when
   missing, reads `dir/placement-identity.json` (`{"node": …, "instance": …}`), or creates it with `node-<16 hex>` and a
   random instance, written atomically. A file that cannot be read or is not that JSON is an error
   (`InvalidIdentity`), never a fresh identity, so a machine never takes a new instance by accident. `node_id()` is
   the default for `NodeConfig::node_id`; `instance()` goes into offers and stays across restarts, so a restart is the
   same machine to the pool. The rustdoc of `Offer::instance` changes from "drawn at each process start" to "kept
   across restarts".
4. **Conflicts are the pool's, in their own change.** Refusing a second machine under a live node's id, with one alert,
   is `placement-conflicts` (before this change): the pool accepts such an offer's turn, ignores the offer and queues a
   `conflict` alert, so the alert and its once-only mark survive (design review, blocking 1 and 4). This change only
   sends offers; until then a second machine replaces the entry, as offers do today.
5. **Additive inside the line.** `PlacementMember`, `MemberSettings`, `Capacity`, `NodeIdentity`, `InvalidIdentity`; it
   ships with the line in 0.24.0.

## Tests

On virtual time with `TestActors`: the member offers each interval while the probe answers and stops when it returns
`None`, and the pool marks the node silent; the offer carries `system.node_id()`; a failed send is retried; `stop` ends
`run`, also while the probe hangs; a zero interval is refused. In a temp folder: `load_or_create` makes the folder and
the file once and reads the same identity after; a corrupt file and an unreadable one give `InvalidIdentity`.

## Docs

The handoff adds the member to the guide's placement pool section: the identity file, `NodeConfig::node_id`, the probe
(Docker's `info()` as the example), running it as a task, and the conflict alert with its limit.
