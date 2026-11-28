Verdict: pass

## Blocking findings

None.

All four tasks are ticked, and the diff holds their work. Each spec scenario has a test: flow caller, actor caller with a later reply and `c1` (plus the turn-inherited case), `5` gives `Invalid` and nothing is buffered, a missing type name gives `SendRejected`, a saved `ReplyTo`, and both forms in one object. The actor send goes through `Turn::send_raw`, so it gets `send_to`'s type-name, NUL, depth and correlation-bound checks. The flow form's behaviour is unchanged (`send_encoded_reply` with `None`). The version is 0.26.0 in Cargo.toml, both skills and plugin.json. ADR 0079 and its row came in ebc09060. The CHANGELOG has the Added line. No protected docs are in the diff. The handoff matches the code, and its status says no prose is written.

## Notes

1. `crates/cicada-actors/src/reply.rs:112`: the actor form's `Invalid` message is the bare `MessageError` text. The flow form says "the reply to request {id} does not encode". A prefix such as "the reply to {to} does not encode" would make target failures easier to read.
2. `crates/cicada-actors/src/reply.rs:207`: `"correlation": null` fails to decode in the actor form, but serde's `Option` normally accepts null. This matches the schema (`type: string`). The rustdoc could say so for hand-written clients.
3. `crates/cicada-actors/src/reply.rs:189-195`: the flow form ignores a `correlation` key without a word, even a non-string one. This follows the design ("unknown fields ignored"). Still, a client that adds `correlation` to a flow route loses it without an error. Worth a line in the rustdoc.
4. `openspec/changes/reply-route/docs-handoff.md:46`: "the contract upgrade procedure" should name the existing section, [Changing a contract](docs/operations.md#changing-a-contract), so the docs session links to it.

## Not checked

I did not run tests, the gate or the doctest. I relied on the builder's recorded gate pass of d8c20cc9. I did not check decoding through formats other than JSON / serde_json `Value` (the custom visitor only implements `visit_map`). There are no boards: this is not a GUI change.
