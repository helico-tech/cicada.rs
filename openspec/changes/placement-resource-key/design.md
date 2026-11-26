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
   fills it from Docker's `GET /info` field `ID`, a second call next to `/version` (an error of either fails `info()`
   as today); the fake reports `None` until `set_info` sets one. Not taken: a separate method (one more call site for
   hosts).
2. **An optional resource key on offers.** `Capacity::resource(key)` and `Offer { …, resource: Option<String> }` (1 to
   200 characters, no NUL). The pool records the key per node.
3. **One live holder per key.** An offer whose key is held by another node that is `live` changes nothing for the
   offering node (a new node is not added; a known one keeps its last entry), and sets that node's conflict mark of ADR
   0069 with the `resource` (the mark gains an optional `resource`) and sends one `conflict` alert per mark. The first
   live holder keeps the key; when it goes silent or is gone, the next offer with the key takes it. A mark with
   `resource` ends one silence time after the last conflicting offer, as ADR 0071's `host` mark.
4. **An ADR.** ADR 0072 extends ADR 0069 and ADR 0071 with the resource rule. Inside the unreleased 0.25.0 line: a
   field of `RuntimeInfo` (additive), of `Offer`, of the node entry and of the mark (a pool contract change).

## Tests

`cicada-containers`: `RuntimeInfo::id` from a mapped `/info`, the fake's `set_info`; the E2E class checks the id is set.
The pool, on virtual time: a second live node offering a held key is ignored, alerted once, and takes the key after the
first goes silent; offers without a key are unaffected; the mark ends one silence time after the last conflicting offer;
contract and samples.

## Docs

The handoff gives the guide's member section the resource key with Docker's id as the example, and the pool section the
rule and its alert.
