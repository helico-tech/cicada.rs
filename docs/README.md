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
| Design a system of actors: boundaries, protocols, recovery | [Design with actors](guide/design.md) |
| Run an actor on one executor | [Pin an actor](guide/core.md#pin-an-actor-to-an-executor) |
| Serve a live socket from durable state | [Websocket session](guide/design.md#a-websocket-session) |
| Send, retry or schedule a message | [Core](guide/core.md#retry) |
| Understand message paths in the examples | [Visual patterns](guide/patterns.md) |
| Test a timer without waiting | [TestActors](guide/core.md#testing-with-testactors) |
| Own child actors or react to failures | [Children](guide/core.md#child-actors), [watches](guide/core.md#watches) |
| Run actors without database credentials | [HTTP worker](guide/app.md#a-worker-over-http) |
| Choose a scheduler, limiter, debouncer or batcher | [Ready-made actors](guide/actors.md) |
| Cap how many jobs run at once | [Permit pool](guide/actors.md#permit-pool) |
| Know which nodes offer room for work | [Placement pool](guide/actors.md#placement-pool) |
| Run a graph of jobs given as data, with cancel and retry | [Job graph](guide/actors.md#job-graph) |
| Write a durable multi-step job | [Flows](guide/actors.md#flows) |
| Run containers from actors or host code | [Containers](guide/containers.md) |
| Wait durably for another actor or a human decision | [Flow requests and replies](guide/actors.md#requests-and-reply-waits) |
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

[Install the consumer skills](how-to/install.md#install-the-coding-agent-skill), or use the [agent reading map](agents.md)
to locate the documentation of your resolved dependency. The `actor-design` skill designs a system with you step
by step, or reviews an existing actor design or implementation, and saves the result as Markdown with diagrams.
Contributors follow [AGENTS.md](../AGENTS.md). The documentation owner follows the
[writing standard](maintaining/writing.md) and [maintenance procedure](maintaining/maintenance.md).
