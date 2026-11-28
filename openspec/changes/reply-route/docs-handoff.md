# Docs handoff for reply-route

Version 0.26.0. Refresh the dependency pins in `docs/how-to/install.md` once it is published.

## What changed

`cicada_actors::ReplyRoute<T>` gives one request field to flows and actors. The rustdoc for `ReplyRoute::send` shows
both callers and the target. The [reply-route tests](../../../crates/cicada-actors/tests/reply_route.rs) show a flow
receiving a reply in `step.wait`, an actor receiving a later reply with its correlation id, inheritance from the
replying turn, and the errors for an unencodable value or a reply type name the caller lacks.

## 1. Requests and reply waits

**Where.** `docs/guide/actors.md`, “Requests and reply waits”, near `step.request`.

**Text.** For a port that also serves actors, declare `reply: ReplyRoute<T>` in its request. A flow's request builder
puts `r.into()` in that field, and `step.wait` returns `T`. An actor puts
`ReplyRoute::actor(turn.self_address().clone()).correlation(id)` in that field. The target uses
`reply.send(turn, &value).await?` for either caller. The flow form carries `run` and `request`, and the actor form
carries `actor` and an optional `correlation` string. The rustdoc example for `ReplyRoute::send` uses `JobReply` as
the port's reply family and shows the flow and actor call sites.

**Caveats.** Set a distinct route correlation id when the caller needs to match several open requests. The id travels
with a stored route into a later turn. With no id on the route, the reply inherits the replying turn's correlation
id. The flow form follows the correlation behavior of `ReplyTo::send`.

## 2. Ports and fakes

**Where.** `docs/guide/design.md`, “Ports and fakes” from the `actor-ports` handoff.

**Text.** A port used by both flows and actors can own one request message with a `ReplyRoute<PortReply>` field.
Its reply family uses `#[serde(tag = "type")]`, and each actor caller's own family accepts the reply variants by name
and fields. Encode each port reply with `message::encode` and decode it with `message::decode` as the caller's
message family to test that match.

**Caveats.** The send checks the reply type name against the caller's registered contract and returns
`TurnError::SendRejected` if the type or message name is missing. The caller decodes the fields when handling the
reply. A reply value without a message type tag gives `TurnError::Invalid` in the target.

## 3. Upgrading a port

**Where.** `docs/how-to/upgrading.md`, “Upgrade to 0.26.0”.

**Text.** Adding `ReplyRoute<T>` to the library leaves existing contracts as they were. Changing a port field from
`ReplyTo<T>` to `ReplyRoute<T>` keeps saved `run` and `request` payloads readable as flow routes, and changes the
port's contract hash. Use the contract upgrade procedure for the participating port type before deploying the new
form.

**Caveats.** Send the actor form after the port accepts the new field schema and its hosts and workers can read it.

## Blocking

None.

## Status

The wire, virtual-time and doctest checks passed in `cargo test -p cicada-actors`. The finishing checks passed:
`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` with
PostgreSQL (4116 passed, 0 failed, including `cicada-examples --test docs`, `--test skill` and its doctests) and
`openspec validate reply-route --strict`. The guide and upgrade wording above is proposed for the docs session; none
of it is written yet.
