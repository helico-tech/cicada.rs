#!/usr/bin/env bash
# `cargo test --workspace` turns every feature of cicada and cicada-app on, so each feature is also built alone, with
# warnings denied: cicada with no feature, only SQLite, only PostgreSQL, only the test support; cicada-app with no
# feature, each store alone and the terminal UI alone. Its own target directory keeps RUSTFLAGS from rebuilding the
# workspace's cache. Run from the repository root (scripts/gate.py, policy full).
set -euo pipefail
export RUSTFLAGS="-D warnings"
export CARGO_TARGET_DIR=target/features
cargo check -p cicada
cargo check -p cicada --features sqlite
cargo check -p cicada --features postgres
cargo check -p cicada --features testing
cargo check -p cicada-app
cargo check -p cicada-app --features sqlite
cargo check -p cicada-app --features postgres
cargo check -p cicada-app --features tui
