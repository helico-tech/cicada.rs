Verdict: not yet

## Blocking

1. **Nothing tells the tray when the worker is Online.** design.md Decision 3 offers two sources, and neither exists. `TurnObserver` sees only turns (`turn_started`/`turn_ended`), not polls or leases, so it has no "first poll". `run_with` is one opaque future: the tray cannot see when `wait_for_server` and `worker.start()` (node.rs `run_worker`) finish. An idle worker would stay `Connecting` forever. Fix: decide on one small public signal in `cicada-app`, e.g. a callback after `worker.start()` succeeds, and add it to What Changes and the ADR.
2. **The `until` of `Stopping` cannot be computed.** The grace period comes from flag, environment, host `NodeConfig` and default, and is resolved inside `run_with` (`settings::resolve_http`). `App` exposes none of it. Fix: let the signal from 1 carry the resolved grace period, or drop `until`.
3. **Turns that end without a commit.** The worker drops an `ObservedTurn` without calling `turn_ended` when the commit does not happen: lease lost, cancelled at the stop, completion refused (observer.rs `Drop for Observed`). Decision 4 handles only `turn_ended`, so such turns stay listed as running for good. Fix: decide what happens on drop (remove the turn, or put it in the ring as "not committed") and add a scenario.

## Notes

- Decision 2 does not say which `env` the tray passes. Tests need to inject `CICADA_CONFIG`. `<name>` in the args, and `App::new("factory")`, do not match the code: `App::new()` takes no name.
- An unreachable server stays `Connecting` for up to 60 s of retries (ADR 0042) before it ends `Failed`. Say so in the scenario.
- `tray-menu-bar` says the crate "is empty" off macOS. That contradicts Decision 1 (a portable model). It also adds "Stop Now" and a checkmark for the selected `--types`, which this model does not provide.
- `dependency-trees.sh` has no "leaf" check. Name the actual edit (add `cicada-tray` to the containers loop).

## Owner questions

- Is a new public readiness hook on `App` acceptable here, or should it be its own change before this one?

## Not checked

The mockup, the gate policy entries for a new crate, and whether a turn can be cancelled while it is being decoded.
