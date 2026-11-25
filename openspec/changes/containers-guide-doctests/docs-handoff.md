# Docs handoff for containers-guide-doctests

Version: none (fast lane, tests only; no release of its own).

## What changed

`docs/guide/containers.md` is now compiled as doctests, like the other guides: `examples/build.rs` expands it into
rustdoc markdown (the `<!-- doctest` setup comments work there too), and `examples/src/lib.rs` has
`ContainersGuideDoctests`. `cicada-containers` is a dev-dependency of `cicada-examples` with the features `testing`
(`FakeRuntime`) and `docker` (`DockerRuntime`), so the page's blocks can use both; blocks that connect to Docker are
`no_run`. The page has no Rust blocks yet, so `cargo test -p cicada-examples --doc` runs nothing new until the docs
session adds them. Users of the library see nothing new.

## 1. Name the new doctest struct in AGENTS.md

**Where.** `AGENTS.md`, `Rules`, the bullet that lists the doctest structs of `cicada-examples`.
**Text.** Add `docs/guide/containers.md` to the files whose Rust blocks are doctests, and `ContainersGuideDoctests` to
the list of structs (after `DesignGuideDoctests`).
**Caveats.** None.

## 2. The page's compiled examples

**Where.** `docs/guide/containers.md`, the paragraph `This page has no compiled examples of its own yet.`
**Text.** The page can now carry compiled blocks (a job, a log follower and a test on the fake, as planned). Each needs
the `cicada_containers::` paths or a hidden `# use cicada_containers::...;` line in its setup comment; Docker connects
use `rust,no_run`. Once blocks exist, the sentence that the page has no compiled examples goes.
**Caveats.** Blocks that run must not need Docker: use `FakeRuntime`.

## Blocking

None.

## Status

`cargo test -p cicada-examples --doc` passes with the page wired (no blocks yet). A temporary block in the struct's doc
comment (not the protected page, not committed) showed a `FakeRuntime` block runs, a `DockerRuntime::connect` block
compiles as `no_run`, and a failing assertion fails. All prose above is proposed text; none is written.
