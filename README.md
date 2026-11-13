# Cicada for Rust — durable actors

Cicada is a small durable actor system for Rust services. An actor is a row of state plus a mailbox in a shared
transactional store (PostgreSQL or SQLite); it occupies memory only while one of its messages is being handled. Every
service that runs an `ActorSystem` is a *node*. Nodes are peers that coordinate only through the store: there is no
coordinator and no cluster protocol.

- **Durable turns.** Claim → load → handle → commit. New state, outgoing messages, creates and the message's outcome
  are written in one transaction; a node that dies mid-turn loses its lease and the turn is re-run elsewhere.
- **One turn at a time per actor**, messages in delivery-time order, delayed messages never early, exactly one
  outcome per message (`done`, `failed`, `cancelled`, `purged`).
- **Typed and checked.** Actor addresses are typed refs with a type name and a key (`sync/acme`); the compiler
  checks that a ref accepts a message, and that a handler covers every message of its family.
- **One HTTP API** on every node as an axum `Router`, described by an OpenAPI document, a typed client, an in-process API,
  `TestActors` on tokio's paused time, and per-message retry policies set by the sender.

## Layers

Each layer uses only the public API of the layers below it (ADR 0018). Use the layer you need and read its guide.

| Layer | Crates | What it gives | Guide |
| --- | --- | --- | --- |
| 0 | `cicada`, `cicada-test`, `cicada-otel` | Actor types, the runtime, the stores (features `sqlite` and `postgres`), watches, retry and retention; `TestActors` on virtual time; an OpenTelemetry observer | [The core](docs/guide/core.md) |
| 1 | `cicada-axum`, `cicada-client` | The HTTP API as an axum `Router`, with the schema check at the door; the typed client | [HTTP and the client](docs/guide/http.md) |
| 2 | `cicada-app` | `App`: a set of actor types as a binary with `serve`, `worker` and the operator commands; the binary `cicada` and its terminal UI | [The app](docs/guide/app.md) |
| 3 | `cicada-actors` | Ready-made actors: a supervisor, dead letters, a topic, a scheduler, a rate limiter, a debouncer, a batcher and an event that starts flows; durable flows; `Schedule` | [Ready-made actors](docs/guide/actors.md) |

`examples/` holds fifteen worked examples, each tested ([the core guide](docs/guide/core.md#examples)), and
`sample-service/` a runnable node on `cicada-app` ([the app guide](docs/guide/app.md#sample-service)).

## Quick start

One actor type and a `main` on `App`. The project depends on `cicada` and on `cicada-app` with the feature `sqlite`
(see [Using a release in another project](#using-a-release-in-another-project)), and on `serde` and `schemars`:

```rust,no_run
use cicada::prelude::*;
use serde::{Deserialize, Serialize};

/// The ref: the type name and the key, one name. `Counter { name: "a".into() }` is `counter/a`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counter {
    pub name: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

/// The messages of the type; the variant name is the type name on the wire.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum CounterMsg {
    Add { amount: i64 },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CounterState {
    #[serde(default)]
    pub total: i64,
}

pub struct CounterActor;

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        match message {
            CounterMsg::Add { amount } => turn.state_mut().total += amount, // stored when the turn commits
        }
        Ok(())
    }
}

fn main() -> std::process::ExitCode {
    cicada_app::App::new().host(CounterActor).run() // serve, worker and the operator commands
}
```

Run a node, then create the actor and send it a message with the operator commands of the same binary, over HTTP:

```sh
cargo run -- serve --store actors.db                     # the node and the HTTP API on 127.0.0.1:8080
export CICADA_SERVER=http://127.0.0.1:8080               # in a second terminal
cargo run -- create actor counter/a --state '{}'
cargo run -- send counter/a --type Add --payload '{"amount": 5}'
cargo run -- get actor counter/a -o json                 # "state": {"total": 5}
```

`serve` has no authentication of its own: before it listens on a network, add a layer
([the app guide](docs/guide/app.md#run-it-as-a-binary)).

## Documentation

| Document | What it covers |
| --- | --- |
| [The core](docs/guide/core.md) | Actor types, messages, turns and state, child actors, watches, control effects, the store and the node, retry, retention, the claim limit, observers, testing, the examples |
| [HTTP and the client](docs/guide/http.md) | The routers, the HTTP API, the schema check, mounting in an axum service, the typed client |
| [The app](docs/guide/app.md) | `App`, `serve` and `worker`, the settings, the command line and the terminal UI, the sample service |
| [Ready-made actors](docs/guide/actors.md) | The supervisor, dead letters, the topic, the scheduler, the rate limiter, the debouncer, the batcher, flows, `Schedule` |
| [Architecture](docs/architecture.md) | The reference for behaviour: the domain language, guarantees G1–G6, store requirements S1–S10, the HTTP contract |
| [Guarantees and their tests](docs/guarantees.md) | Each of G1–G6 and S1–S10 with the tests that prove it; checked by a test |
| [Operations](docs/operations.md) | Sizing, deploying, upgrading, releasing, on call, store tuning, telemetry |
| [Command line](docs/reference/cli.md) | The help of every command of `cicada`, and of `serve` and `worker`; generated and checked by a test |
| [Contracts](docs/reference/contracts/) | The contracts of the ready-made actors, a wire format; checked by a test |
| [OpenAPI](docs/openapi.json) | The OpenAPI document of the HTTP API; checked by a test |
| [Decisions](docs/adr/README.md) | The ADRs, one per decision |
| [Changelog](CHANGELOG.md) | What changed in each release |

The Rust blocks of the README, the guides and the operations guide are doctests of the `examples` crate
(`cargo test -p cicada-examples --doc`), so they keep compiling, and the ones that need no server or database run.

## Using a release in another project

A release is a git tag `vX.Y.Z` with a GitHub Release; [CHANGELOG.md](CHANGELOG.md) says what changed. The crates are not
published to a registry (`publish = false`), so they are git dependencies. Name the tag once:

```toml
[workspace.dependencies]
cicada = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.16.12", features = ["postgres"] }
cicada-axum = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.16.12" }
cicada-test = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.16.12" }
```

Name the store's feature on `cicada`: `postgres`, `sqlite`, or both. None is on by default, and each builds only its
own driver; without one the crate builds and opens no store. `cicada-test` turns on the test support itself. A host
that runs on `App` adds `cicada-app` from the same tag in the same way, with the feature of its store (and `tui` for
the terminal UI): its features turn on those of `cicada`.

Every Cicada crate must come from the same tag; mixing tags gives two copies of the same types. In a project that is
not a workspace, put the same lines in `[dependencies]`, and `cicada-test` in `[dev-dependencies]`, with
`tokio = { version = "1", features = ["test-util"] }` next to it. To upgrade, change the tag and read the changelog.

While the version is 0.x, a breaking change raises the minor number (`0.3.1` to `0.4.0`) and anything else the patch
number. A breaking change is one to the public Rust API, the HTTP wire format, the store schema, a contract hash, or a
guarantee in [docs/architecture.md](docs/architecture.md). A published tag never moves.

**Access.** The repository is private, and Cargo must use the git command line to reach it. Keep the URL as it is above
everywhere: Cargo treats another URL as another source.

On a developer machine, in `~/.cargo/config.toml`:

```toml
[net]
git-fetch-with-cli = true
```

and once, `gh auth login` followed by `gh auth setup-git`.

In GitHub Actions, the default `GITHUB_TOKEN` only reaches its own repository. Create a fine-grained token with read
access to the contents of this repository only, keep it as a secret, and use it for this repository only:

```yaml
env:
  CARGO_NET_GIT_FETCH_WITH_CLI: "true"
steps:
  - uses: actions/checkout@v7
  - run: git config --global url."https://x-access-token:${CICADA_TOKEN}@github.com/helico-tech/cicada.rs".insteadOf "https://github.com/helico-tech/cicada.rs"
    env:
      CICADA_TOKEN: ${{ secrets.CICADA_READ_TOKEN }}
```

In a Docker build, pass the same token as a BuildKit secret and set it through the environment of that one step, so
that it does not end up in an image layer. The image needs `git`:

```dockerfile
RUN --mount=type=secret,id=cicada_token \
    GIT_CONFIG_COUNT=1 \
    GIT_CONFIG_KEY_0="url.https://x-access-token:$(cat /run/secrets/cicada_token)@github.com/helico-tech/cicada.rs.insteadOf" \
    GIT_CONFIG_VALUE_0="https://github.com/helico-tech/cicada.rs" \
    CARGO_NET_GIT_FETCH_WITH_CLI=true \
    cargo build --release --locked
```

and build with `docker build --secret id=cicada_token,env=CICADA_TOKEN .`.

## Building and testing

Requirements: Rust 1.94 or newer and, for the PostgreSQL tests, Docker.

```sh
cargo build --workspace
cargo test --workspace                     # every test that needs no database or Docker; every feature on
docker compose up -d postgres              # PostgreSQL on port 55432
CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada
```

[AGENTS.md](AGENTS.md) has every command, the map of the repository and its rules.
