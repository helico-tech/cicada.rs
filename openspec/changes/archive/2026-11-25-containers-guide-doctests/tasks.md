# Tasks

Fast lane, tests only: version `none`.

- [x] 1. **Wire the page.** Add `("docs/guide/containers.md", "guide-containers.md")` to `examples/build.rs`, and
  `ContainersGuideDoctests` to `examples/src/lib.rs` like `ActorsGuideDoctests`. Add `cicada-containers` with the
  features `testing` and `docker` to the dev-dependencies of `examples/Cargo.toml`. Verify:
  `cargo test -p cicada-examples --doc` passes (the page has no Rust blocks yet), and a temporary local Rust block in
  the page compiles and fails as expected (not committed).
- [x] 2. **Handoff and finishing checks.** `docs-handoff.md` asks the docs session to add `ContainersGuideDoctests` to
  AGENTS.md's list of doctest structs; nothing blocks. `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets`, `cargo test -p cicada-examples`, commit, then `python3 scripts/gate.py run`.
