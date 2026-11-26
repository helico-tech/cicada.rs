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
        identity_slot.set(identity).ok();   // kept for the member's task; one run per process
        Ok(config)
    } else {
        Ok(NodeConfig::default())
    }
})
```

## Decisions

1. **A second builder method.** `App::node_config_for(f)`, with
   `f: FnOnce(&[&'static str]) -> anyhow::Result<NodeConfig> + Send + 'static`, synchronous (a host's work there, such
   as loading the placement identity, is plain file I/O). `serve` and `worker` call it once, with the names of the
   types this run hosts in the binary's order; its config then takes the place of the one `node_config` gives. `node_config` and `node_config_for` set one slot: the last call wins. The operator
   commands, `--help` and `--version` never call it. Not taken: a hook that gets and returns the resolved config after
   the flags (the host could then override a flag, which breaks the rule that a flag wins).
2. **Flags still win.** The flags and their `CICADA_…` variables override fields of the returned config exactly as they
   override the host's `NodeConfig` today: `--node-id` replaces the node id the hook chose.
3. **A worker over HTTP.** It calls the hook the same way with its types and takes from the config only what it takes
   today from the host's (`worker_id`, concurrency, poll interval, longest turn, grace period, cancel timeout).
4. **The hook runs after every local check.** It has side effects (the identity's file lock), so it runs only once
   nothing that reads the flags, the variables or the config file can still fail: after the usage errors (exit code
   2), the refusal of `on_start` and `task` for a worker over HTTP, `settings::http_target` (the config file, the
   context, the server) and the check that the binary was built with the store's feature (called from `lib.rs` on the
   parsed store, before the hook, instead of inside `open_unless_stopped`, so there is one path; a worker over HTTP has
   no store and skips it); just before the wait for the store or the server. `settings::resolve` and `resolve_http` are split into a parse step (every flag and variable
   read and checked, with the types) and an apply step that lays the parsed values over a `NodeConfig`; the hook runs
   between them, so no flag error can show after it ran. An `Err` from `f` fails the command with exit code 1 and the
   error's text (with its causes); a panic is not caught.
5. **What the guide says.** The hook runs once per `run`; a host that keeps state from it (the identity for the
   member's task) keeps it for one run, so a process that runs `App` more than once (a tray, tests) builds a fresh
   slot each time. With `--node-id other` the hook still runs, and an identity it loads differs from the node id, as
   with `node_config` today.

## Tests

The resolved node id with `node_config_for` choosing by type: all types, `--types` without the type, and `--node-id`
overriding it; the hook is called once with the picked types in the binary's order, and not for an operator command;
the last of `node_config` and `node_config_for` wins; an `Err` fails with exit code 1 and the message, and a bad flag
fails with exit code 2 without calling it, as do a bad `--context` (exit code 1) and a store whose feature is not built;
the worker over HTTP takes its id from the hook.

## Docs

The handoff gives the app guide's settings section `node_config_for` with the placement identity as the example and
decision 5's notes, and the changelog's 0.25.0 section the method under Added. The builder updates the module rustdoc
of `cicada-app` (the order of the settings) and `App`'s `Debug`.
