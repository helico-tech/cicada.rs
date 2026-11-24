Verdict: fix needed

## Blocking

1. `crates/cicada-containers/src/actor/mod.rs:237`: the `Start` handler saves `message_type` in `state.target`
   unchecked, but the state decoder checks it (`wire.rs:296`). A bad `Start` saves a state no node can decode, and the
   actor fails every later message. The HTTP door's schema (1–200 characters) protects only HTTP; code sends are not
   checked (ADR 0026), serde ignores the schemars lengths, and the variant is public (`tests/actor.rs:341` builds it).
   How to see it: send `ContainerMsg::Start { to, message_type: String::new() }` (or 201 characters) in process, then
   any message; its turn cannot decode the state.
   Fixed: the handler (or the message decoder) refuses a `Start` that fails `check_message_type`: the turn fails
   permanently with the reason and nothing is saved. A test shows the actor still works after it.

## Notes

1. `mod.rs:465` (`logs`): the inspect that just succeeded does not reset `errors`, against the design's "any success
   resets `errors`". After 9 failed inspects, one failed `logs` saves `(no logs: …)` at once. Reset after a
   successful inspect in `follow` and `stop`.
2. A `Start` after the end sends the end again every time (the builder's stated choice). The actors guide should say
   the target can get the same end more than once.
3. Tasks are ticked, every spec scenario has a test, the handoff matches the code, and no protected docs were edited.

## Not checked

No build or test run; I relied on the builder's gate evidence for 9e531e3. Finding 1 comes from reading the code and
ADR 0026, not from a running test. The contract fixture only through its test. Nothing against a real Docker daemon.
