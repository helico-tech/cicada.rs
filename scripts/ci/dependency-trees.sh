#!/usr/bin/env bash
# The dependency trees each feature must and must not build. Run from the repository root (scripts/gate.py, policy
# full). The trees go to files first, so a `cargo tree` that fails fails the script; the positive greps check that the
# trees hold what each feature must build.
#
# - cicada-app without tui builds no terminal library; with tui it builds ratatui.
# - Each store feature builds only its own driver, and SQLite builds no TLS library.
# - The whole workspace, on every target and with its dev-dependencies, builds rustls with one crypto provider: with
#   both ring and aws-lc-rs, every caller that leaves the choice to rustls panics.
set -euo pipefail
trees=$(mktemp -d)
trap 'rm -rf "$trees"' EXIT

cargo tree -p cicada-app -e normal > "$trees/app.txt"
cargo tree -p cicada-app -e normal --features tui > "$trees/app-tui.txt"
grep -q 'ratatui v' "$trees/app-tui.txt" || { echo "tui does not build ratatui"; exit 1; }
if grep -qE 'ratatui|crossterm' "$trees/app.txt"; then echo "cicada-app without tui builds ratatui or crossterm"; exit 1; fi

cargo tree -p cicada -e normal > "$trees/none.txt"
cargo tree -p cicada -e normal --features sqlite > "$trees/sqlite.txt"
cargo tree -p cicada -e normal --features postgres > "$trees/postgres.txt"
cargo tree --workspace --target all -f '{p} [{f}]' --prefix none > "$trees/workspace.txt"
grep -q sqlx-sqlite "$trees/sqlite.txt" || { echo "sqlite does not build the SQLite driver"; exit 1; }
grep -q sqlx-postgres "$trees/postgres.txt" || { echo "postgres does not build the PostgreSQL driver"; exit 1; }
if grep -q sqlx "$trees/none.txt"; then echo "no feature builds sqlx"; exit 1; fi
if grep -q sqlx-postgres "$trees/sqlite.txt"; then echo "sqlite builds the PostgreSQL driver"; exit 1; fi
if grep -qE 'sqlx-sqlite|libsqlite3-sys' "$trees/postgres.txt"; then echo "postgres builds the SQLite driver"; exit 1; fi
if grep -qE 'rustls|native-tls' "$trees/sqlite.txt"; then echo "sqlite builds a TLS library"; exit 1; fi
if awk '/^rustls v/ && /[[,]ring[],]/ && /[[,]aws_lc_rs[],]/ { two = 1 } END { exit !two }' "$trees/workspace.txt"; then
    echo "the workspace builds rustls with two crypto providers, ring and aws-lc-rs"; exit 1
fi
