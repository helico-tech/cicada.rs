# Design

Two runner nodes on one machine, each with its own identity, both offer the same Docker daemon, so its places count
twice (Factory epic 0008; blocks its merge). An offer can name the resource it counts, and the pool lets one live node
hold a resource at a time.

```rust
let info = runtime.info().await.ok()?;
Some(Capacity::new([info.arch.as_str(), "docker"], 4).resource(info.id.as_deref()?))   // the daemon's id
```

## Decisions

1. **The daemon's id in `RuntimeInfo`.** `RuntimeInfo` (`#[non_exhaustive]`) gains `id: Option<String>`; `DockerRuntime`
   fills it from Docker's `GET /info` field `ID` (empty: `None`), a second call next to `/version` (an error of either
   fails `info()` as today; `/info` can be slow on a busy daemon and now runs at every probe, the rustdoc says so);
   `RuntimeInfo::with_id(id)` sets it from other crates; the fake reports `None` until `set_info` sets one. Not taken: a
   separate method (one more call site for hosts).
2. **An optional resource key on offers.** `Capacity::resource(key)` and `Offer { …, resource: Option<String> }` (1 to
   200 characters, no NUL). The pool records the key per node.
3. **One live holder per key; contests in a list of their own.** An offer whose key a different `live` node holds
   changes nothing for the offering node (a new node is not added; a known one keeps its last entry and goes silent if
   it keeps offering only the held key). The pool records the contest in a pool-level list `contested: [{resource,
   node, at}]` (at most `maxNodes` entries), one entry per key and offering node, `at` the store time of the last such
   offer, and sends one `conflict` alert (naming the node; the state holds the key) when an entry is added. The check
   timer removes an entry one silence time after its `at`. The node entries and ADR 0069's mark do not change, so its
   shape and decode stay as they are (design review: the mark had no entry to live on). The first live holder keeps
   the key; when it goes silent or is gone, the next offer with the key takes it (its runs may still use the resource
   for a while: a brief overbooking, in the rustdoc). The decode refuses two live nodes holding one key.
4. **An ADR.** ADR 0072 extends ADR 0069 and ADR 0071 with the resource rule. Inside the unreleased 0.25.0 line: a
   field of `RuntimeInfo` (additive), of `Offer`, of the node entry and of the mark (a pool contract change).

## Tests

`cicada-containers`: `RuntimeInfo::id` from a mapped `/info`, the fake's `set_info`; the E2E class checks the id is set.
The pool, on virtual time: a new node offering a held key is not added and alerted once; a known node is ignored and
alerted once even after it goes silent and offers again within the silence time; the contest ends one silence time
after the last conflicting offer and a later one alerts again; the second takes the key after the first goes silent;
offers without a key are unaffected; a decoded state with two live holders of one key is refused; contract and samples.

## Docs

The handoff gives the guide's member section the resource key with Docker's id as the example, and the pool section the
rule and its alert.
