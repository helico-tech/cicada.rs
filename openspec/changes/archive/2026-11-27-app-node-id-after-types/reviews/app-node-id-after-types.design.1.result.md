Verdict: not yet

## Blocking

1. **Where the hook runs among the checks after the flags is not decided** (design.md decisions 1 and 4, spec requirement). Two checks that read only local input fail with exit code 1 after `resolve`/`resolve_http` today: `settings::http_target` (config file, unknown context, no server; lib.rs Worker/Http branch) and `check_built_in` (a store feature the binary lacks; node.rs `open_unless_stopped`). "Before the store opens or a request is sent" fits both orders. The hook has side effects: it takes the identity's file lock. So a bad `--context`, which is the tray's "bad context" case, would take the lock and then fail. What I would do: decide that the hook runs after every check that reads only flags, variables and the config file (usage errors, the HTTP refusal of on_start/task, `http_target`, `check_built_in`), and just before the store wait or the server wait. Add spec scenarios: "a bad flag: exit 2, hook not called" and "an operator command does not call it". Today these are only in the Tests list, not in the spec.

## Notes

1. `resolve` merges the host config while it parses the flags (lib.rs/settings.rs), so "flags read first, then the hook" means splitting `resolve` into a parse step and an apply step, or calling it twice. Say which, so that a flag error never shows up after the hook.
2. The example's `identity_slot.set(..).ok()` works for one run per process only. With the tray (a fresh `App` per connect, tray-worker-control) or `run_with` in tests, the second load fails `Locked`, or the old identity stays in the slot. The handoff example should point this out. Otherwise no conflict with tray-worker-control: `type_names` and `on_worker_ready` only touch the struct, `new` and `Debug` as text.
3. `--node-id other` still runs the hook, which still takes the lock. The member's identity then differs from the node id. This is how it works today, but the guide should say it.
4. The module rustdoc (lib.rs line 42: "else the host's App::node_config") and the `App` `Debug` impl change too. Name them in the tasks.
5. A sync hook is enough (`load_or_create` does sync file I/O). Say why async is not needed.

## Questions for the owner

None.

## Not checked

Factory's own code. I did not build or run anything.
