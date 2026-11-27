# Docs handoff for app-node-id-after-types

Version: 0.25.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`App::node_config_for` supplies the host's `NodeConfig` through a synchronous function of the types this run hosts.
The function receives names in the order of the `App::host` calls. It runs once for `serve` or `worker` after local
checks and before the store or server wait. A returned error and its causes appear in the command's exit-code-1
diagnostic.

`crates/cicada-app/tests/app.rs` checks the default and selected types, the node id used at startup, flag
precedence, the last builder call, errors and commands that skip the function. `crates/cicada-app/tests/worker_http.rs`
checks that a worker claims a turn pinned to the id returned by the function and that invalid HTTP targets and
refused startup callbacks skip it. The unit test in `crates/cicada-app/src/lib.rs` checks the missing-store-feature
path.

## 1. Choose the node config from the hosted types

**Where.** `docs/guide/app.md`, `Settings`, after the settings precedence paragraph.

**Text.** Use `App::node_config_for` when a node setting depends on the types selected by `--types` or
`CICADA_TYPES`. For example, load a placement identity only on the runs that host the runner. A serve-only run
started with `--types none` receives an empty slice. The last call to either `node_config` or `node_config_for`
sets the host config. A flag wins over its environment variable, and either wins over the corresponding field of
the host config. An HTTP worker uses the same subset of fields as it does with `node_config`.

The callback runs once after the local startup checks and before waiting for the store or the server. Operator
commands, help and version skip it. A returned error fails the command with exit code 1.

Proposed `AppGuideDoctests` example for `Settings`. The type selection pattern comes from
`node_config_for_sees_the_selected_types_in_host_order_and_flags_win` in `crates/cicada-app/tests/app.rs`.

```rust
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use cicada::runtime::NodeConfig;
use cicada_actors::placement::NodeIdentity;
use cicada_app::App;

fn with_runner_identity(app: App, dir: PathBuf, identity_slot: Arc<OnceLock<NodeIdentity>>) -> App {
    app.node_config_for(move |types| {
        if types.contains(&"factory.runner") {
            let identity = NodeIdentity::load_or_create(&dir)?;
            let config = NodeConfig { node_id: identity.node_id(), ..NodeConfig::default() };
            identity_slot.set(identity).map_err(|_| anyhow::anyhow!("runner identity was already set"))?;
            Ok(config)
        } else {
            Ok(NodeConfig::default())
        }
    })
}
```

**Caveats.** Use a fresh slot for each run if a process builds a fresh `App` to run again. Share the slot with
the placement member's task for that run and release the identity afterward so the next run can lock the folder.
With `--node-id other` the callback still loads and locks the identity when the runner is selected. The node uses
`other` and the placement member keeps the id of the loaded identity.

## Blocking

None.

## Status

`cargo test -p cicada-examples --test docs` passed 11 tests and `cargo test -p cicada-examples --doc` passed 51.
The app guide text and its example are proposed for the docs session. The changelog entry is in the 0.25.0 batch.
