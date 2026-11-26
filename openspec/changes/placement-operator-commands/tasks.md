# Tasks

Fast lane, tests only: no version change (the 0.24.0 placement release carries it).

- [x] 1. **The recipes, tested.** A test that hosts a placement pool behind the HTTP API (`cicada-axum`) and posts the
  documented payloads to the send route the `cicada send` command uses: `Drain` and `Undrain` of a node, `Gone` of a
  silent node, `Release` of a holder's grant; it checks the pool's state after each. Verify: `cargo test -p
  cicada-actors`.
- [ ] 2. **Docs handoff and finishing checks.** `docs-handoff.md` carries the four `cicada send` recipes, how to read
  the
  pool's state from the command line, and why there is no dedicated command; nothing blocks. `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets`, commit, then `python3 scripts/gate.py run` (in the background with a 2-hour
  timeout).
