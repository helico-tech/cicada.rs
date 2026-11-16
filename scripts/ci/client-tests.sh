#!/usr/bin/env bash
# The JavaScript tests of the example browser clients, with Node's built-in test runner and no packages. Run from the
# repository root (scripts/gate.py, policy full). Node 26 or later must be the `node` on PATH: a missing or older node
# fails the check, it never skips.
set -euo pipefail
if ! command -v node > /dev/null; then
    echo "client-tests: node not found on PATH; install Node 26 or later" >&2
    exit 1
fi
major=$(node -p 'process.versions.node.split(".")[0]')
if [ "$major" -lt 26 ]; then
    echo "client-tests: node $(node --version) is older than 26; install Node 26 or later" >&2
    exit 1
fi
node --test examples/tests/websocket_session_client.test.mjs
