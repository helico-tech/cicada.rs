# Design

A host gives `App::node_config(config)` before the command line is read, so it cannot know whether this run hosts a
type. Factory's binary sets the runner's placement identity as the node id; a node started with `--types` that leave out
the container type still takes that id (and its file lock), instead of a node id of its own (Factory issue 0022, epic
0008). The host should choose the node config once the types of the run are known.

```rust
app.node_config_for(|types| {
    if types.contains(&"factory.runner") {
        let identity = NodeIdentity::load_or_create(&dir)?;
        let config = NodeConfig { node_id: identity.node_id(), ..NodeConfig::default() };
        identity_slot.set(identity).ok();   // kept for the member's task
        Ok(config)
    } else {
        Ok(NodeConfig::default())
    }
})
```

## Decisions

1. **A second builder method.** `App::node_config_for(f)`, with
   `f: FnOnce(&[&'static str]) -> anyhow::Result<NodeConfig> + Send + 'static`. `serve` and `worker` call it once,
   after the store, the types (`--types`, `CICADA_TYPES`, else all hosted) and the other flags are read and before the
   store opens, with the names of the types this run hosts in the binary's order; its config then takes the place of
   the one `node_config` gives. `node_config` and `node_config_for` set one slot: the last call wins. The operator
   commands, `--help` and `--version` never call it. Not taken: a hook that gets and returns the resolved config after
   the flags (the host could then override a flag, which breaks the rule that a flag wins).
2. **Flags still win.** The flags and their `CICADA_…` variables override fields of the returned config exactly as they
   override the host's `NodeConfig` today: `--node-id` replaces the node id the hook chose.
3. **A worker over HTTP.** It calls the hook the same way with its types and takes from the config only what it takes
   today from the host's (`worker_id`, concurrency, poll interval, longest turn, grace period, cancel timeout).
4. **An error fails the command.** An `Err` from `f` fails `serve` or `worker` with exit code 1 and the error's text
   (with its causes), before the store opens or a request is sent; a panic is not caught. A usage error in the flags is
   reported first (exit code 2) and the hook is not called.

## Tests

The resolved node id with `node_config_for` choosing by type: all types, `--types` without the type, and `--node-id`
overriding it; the hook is called once with the picked types in the binary's order, and not for an operator command;
the last of `node_config` and `node_config_for` wins; an `Err` fails with exit code 1 and the message, and a bad flag
fails with exit code 2 without calling it; the worker over HTTP takes its id from the hook.

## Docs

The handoff gives the app guide's settings section `node_config_for` with the placement identity as the example, and
the changelog's 0.25.0 section the method under Added.
