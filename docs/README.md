# Cicada documentation

Cicada runs durable actors inside Rust services. Choose a task below; you do not need to read every layer.
The guides assume you know Rust, serde and async functions.

## Start

1. [Install the crates](how-to/install.md).
2. [Build your first actor](guide/core.md#define-an-actor-type).
3. [Run it as a binary](guide/app.md#run-it-as-a-binary), or [embed it in axum](guide/http.md#run-it-in-an-axum-service).

## Solve a task

| Task | Read |
| --- | --- |
| Send, retry or schedule a message | [Core](guide/core.md#retry) |
| Understand message paths in the examples | [Visual patterns](guide/patterns.md) |
| Test a timer without waiting | [TestActors](guide/core.md#testing-with-testactors) |
| Own child actors or react to failures | [Children](guide/core.md#child-actors), [watches](guide/core.md#watches) |
| Run actors without database credentials | [HTTP worker](guide/app.md#a-worker-over-http) |
| Choose a scheduler, limiter, debouncer or batcher | [Ready-made actors](guide/actors.md) |
| Write a durable multi-step job | [Flows](guide/actors.md#flows) |
| Build a live view of actor state | [Change hints](guide/http.md#follow-changes-over-http) |
| Deploy, diagnose or tune a node | [Operations](operations.md) |
| Upgrade a version or contract | [Upgrade](how-to/upgrading.md), [contract changes](operations.md#changing-a-contract) |
| Publish a release | [Release procedure](how-to/releasing.md) |

## Look something up

| Reference | Authority |
| --- | --- |
| [Architecture](architecture.md) | Current behavior, guarantees, turn and HTTP semantics |
| [Guarantees and tests](guarantees.md) | Tests proving G1–G6 and S1–S10 |
| [CLI](reference/cli.md) | Generated command help |
| [Actor contracts](reference/contracts/) | Checked state and message wire formats |
| [OpenAPI](openapi.json) | Generated HTTP schema |
| [ADRs](adr/README.md) | Decisions and their rationale |
| [Examples](guide/core.md#examples) | Complete implementations with tests |

Generate the Rust API reference locally with `cargo doc --workspace --no-deps`.

## Work with an agent

Use the [agent reading map](agents.md) to locate the documentation of your resolved dependency.
Contributors follow [AGENTS.md](../AGENTS.md). The documentation owner follows the
[writing standard](maintaining/writing.md) and [maintenance procedure](maintaining/maintenance.md).
