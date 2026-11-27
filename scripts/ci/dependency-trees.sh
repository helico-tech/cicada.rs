#!/usr/bin/env bash
# The dependency trees each feature must and must not build. Run from the repository root (scripts/gate.py, policy
# full). The trees go to files first, so a `cargo tree` that fails fails the script; the positive greps check that the
# trees hold what each feature must build.
#
# - cicada-app without tui builds no terminal library; with tui it builds ratatui.
# - Each store feature builds only its own driver, and SQLite builds no TLS library.
# - The whole workspace, on every target and with its dev-dependencies, builds rustls with one crypto provider: with
#   both ring and aws-lc-rs, every caller that leaves the choice to rustls panics.
# - No other library crate has cicada-containers, cicada-tray or cicada-inspector in its tree, with every feature and
#   its dev-dependencies (ADRs 0056, 0074 and 0076). The one exception: cicada-tray may build cicada-inspector, through
#   its optional feature inspector (ADR 0076).
# - cicada-containers without features builds no Docker client; with docker it builds bollard and no TLS library.
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

for crate in cicada cicada-test cicada-axum cicada-client cicada-otel cicada-actors cicada-app cicada-containers \
    cicada-tray cicada-inspector; do
    cargo tree -p "$crate" --all-features --target all --prefix none > "$trees/lib-$crate.txt"
    grep -q "^$crate v" "$trees/lib-$crate.txt" || { echo "the tree of $crate does not hold $crate"; exit 1; }
    for application_crate in cicada-containers cicada-tray cicada-inspector; do
        if [[ $crate == cicada-tray && $application_crate == cicada-inspector ]]; then continue; fi
        if [[ $crate != "$application_crate" ]] && grep -q "^$application_crate v" "$trees/lib-$crate.txt"; then
            echo "$crate builds $application_crate"
            exit 1
        fi
    done
done

cargo tree -p cicada-containers -e normal --target all > "$trees/containers.txt"
cargo tree -p cicada-containers -e normal --target all --features docker > "$trees/containers-docker.txt"
grep -q 'bollard v' "$trees/containers-docker.txt" || { echo "docker does not build bollard"; exit 1; }
if grep -q bollard "$trees/containers.txt"; then echo "cicada-containers without docker builds bollard"; exit 1; fi
if grep -qE 'rustls|native-tls' "$trees/containers-docker.txt"; then echo "cicada-containers with docker builds a TLS library"; exit 1; fi
