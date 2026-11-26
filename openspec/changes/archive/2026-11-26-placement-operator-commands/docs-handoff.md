# Docs handoff for placement-operator-commands

Version: none. The 0.24.0 placement release carries this change; it has no changelog entry and changes no crate.
Refresh the dependency pins of `docs/how-to/install.md` once 0.24.0 is published.

## What changed

No new code or command. A test proves the operator's command-line recipes for a placement pool:
`the_documented_cicada_send_payloads_drain_undrain_release_and_declare_gone` in
`crates/cicada-actors/tests/placement.rs` posts the exact `type` and `payload` that `cicada send` sends to
`POST /actors/cicada.placement/runners/messages` (the route the command uses) for `Drain`, `Release`, `Undrain` and
`Gone`, each answered `202`, and reads the pool's state back with `GET /actors/cicada.placement/runners` (what
`cicada describe actor … -o json` prints) after each.

## 1. Operating a pool from the command line

**Where.** `docs/guide/actors.md`, `Placement pool`, after the paragraphs under `Taking a node out of service` (or as
its own short block there); optionally a row or pointer in `docs/operations.md`.

**Text.** An operator drains, undrains, declares gone and releases with the generic `cicada send`; the pool's address
is `cicada.placement/<name>`:

```sh
cicada send cicada.placement/runners --type Drain   --payload '{"node": "mac-1"}'
cicada send cicada.placement/runners --type Undrain --payload '{"node": "mac-1"}'
cicada send cicada.placement/runners --type Gone    --payload '{"node": "mac-1"}'    # only once the node is silent
cicada send cicada.placement/runners --type Release --payload '{"holder": "r1", "grant": 1}'
```

Read the pool with `cicada describe actor cicada.placement/runners` (YAML; `-o json` for JSON), or
`cicada get actor …`, or the terminal UI. Its `state` shows `nodes` (per node id: `status` `live` or `silent`,
`draining`, `places`, `labels`, `heardAt`, `conflict`), `holders` (`holder`, `grant`, `node`) and `waiting`, the line
in order. A `Release` takes the holder id and grant number from `holders`.

Why there is no `cicada placement …` command: `cicada-app` does not depend on `cicada-actors`, and the other
ready-made actors are operated the same way, through `cicada send` and the actor read commands. The command sends the
same message as `PlacementMsg::drain(…)` and the other constructors in Rust.

**Caveats.** The command answers `message … sent` once the message is accepted, not when the pool has handled it: a
`Gone` of a live node or a `Drain` of an unknown node is refused for good later, as a failed message of the pool (see
`cicada get outcomes --status failed`). A `Release` by an operator frees a place the holder may still use: release
only a grant whose work you know has ended. The same rules hold as for the Rust constructors (a node id 1 through 200
characters, not blank, without NUL; a payload that does not match the type's schema is refused with `422`).

## Blocking

None.

## Status

`cargo test -p cicada-actors --test placement`: 45 passed, 0 failed. The text above is proposed; no prose is written
yet.
