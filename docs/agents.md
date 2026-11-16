# Agent reading map

Use the documentation from the **resolved Cicada dependency**, not a newer checkout or an unrelated installed skill.
This page routes consumer tasks; repository contributors also follow [AGENTS.md](../AGENTS.md).

## Resolve the source first

The [consumer skill](how-to/install.md#install-the-coding-agent-skill) automates this discovery.
With its Python helper, pass the project's actual target/features and follow the returned source groups.

Run `cargo metadata --locked --offline --format-version 1` in the consumer project with its actual target/features.
Find the exact Cicada package entries and use their `manifest_path` directories. Report missing sources, multiple
versions, path dependencies or local modifications; do not guess a Cargo cache path or fetch/upgrade implicitly.
If the dependency tree lacks these docs, use that version’s Rust source/rustdoc and state the limitation.

## Read only what the task needs

| Task | Start here | Implementation evidence |
| --- | --- | --- |
| Define or test an actor | [Core](guide/core.md#define-an-actor-type) | [Actor](../crates/cicada/src/actor.rs), [examples](guide/core.md#examples) |
| Design actors and protocols | [Design with actors](guide/design.md); `actor-design` walks through its nine steps with you | [Websocket session](../examples/src/websocket_session.rs) |
| Design keys/state/turns | [Actor design](guide/core.md#design-an-actor) | [Turn](../crates/cicada/src/turn.rs) |
| Integrate an HTTP service/client | [HTTP](guide/http.md) | [OpenAPI](openapi.json), [client](../crates/cicada-client/src/lib.rs) |
| Run App or a remote worker | [App](guide/app.md) | [CLI reference](reference/cli.md), [remote worker example](../examples/src/remote_worker.rs) |
| Choose a ready-made actor | [Pattern chooser](guide/actors.md) | [Contracts](reference/contracts/) |
| Write/change a flow | [Flows](guide/actors.md#flows) | [Flow source](../crates/cicada-actors/src/flow.rs) |
| Diagnose/deploy/upgrade | [Operations](operations.md) | [Upgrade procedure](how-to/upgrading.md) |
| Change runtime behavior | [Architecture](architecture.md), relevant [ADRs](adr/README.md) | [Guarantee tests](guarantees.md) |

Use exact headings and symbol names to search. Follow a link for deeper detail; loading every guide is unnecessary.
A missing heading is a reason to inspect that version, not evidence that an API is absent.

## Rules that prevent expensive mistakes

- Create actors before sending. Sending does not create them.
- Buffer effects through Turn; external calls need idempotency keys because execution can repeat.
- The sender chooses handler retry; crash recovery and claim limits are separate mechanisms.
- Check contract snapshots and old state samples before changing a public type. A Rust-compatible change can change its hash.
- Flow code outside steps replays. Keep it deterministic, keep step ids stable and never reuse run keys for new jobs.
- A remote completion can apply without an answer. Repeat the current report, never a finished handler.
- Verify changed examples; do not substitute pseudocode for a compiling implementation.

For contributors, architecture defines current behavior; accepted ADRs explain decisions, while an open proposal is
future work. Generated references come from their generators. The [writing standard](maintaining/writing.md)
explains how to keep this shared documentation useful to both people and agents.
