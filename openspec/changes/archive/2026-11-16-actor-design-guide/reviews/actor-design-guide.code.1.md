---
id: actor-design-guide.code.1
kind: code
change: actor-design-guide
round: 1
base: 6e589e6c1c33d3269376d6c082603a282e799f40
head: 6e69ae7f05390b050f466674410b8982fe529393
requested: 2026-11-16T13:33:28+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-design-guide.code.1 <file>`.

## Note from the requester

Version: 0.19.2

Builder report (feat/actor-design-guide, head 6e69ae7f0539, base 6e589e6; not yet rebased onto main, which has moved by design/docs commits only):

Tasks: all six done.
1. Session and producer actors (examples/src/websocket_session.rs), protocol tests on virtual time.
2. Host bridge, axum route, runnable demo and browser client (examples/src/websocket_session/, examples/examples/websocket_session.rs and .html).
3. Recovery tests (examples/tests/websocket_session.rs).
4. Guide, skill routes and doctest wiring (DesignGuideDoctests in examples/build.rs and examples/src/lib.rs).
5. Version 0.19.2 (workspace, Cargo.lock, skill, plugin.json, openapi.json), changelog, handoff; ADR 0050 unchanged.
6. Finishing.

Checks on 6e69ae7:
- cargo test -p cicada-examples --doc: 30 passed. --test docs: 11. --test skill: 10. --test websocket_session: 26 (stable over 40 repeat runs earlier).
- openspec validate actor-design-guide --strict: valid.
- gate.py run --e2e for this commit: full pass and e2e pass (gate.py status confirms; docs record missing, expected for a builder).
- Demo run by hand: HTTP 101 upgrade, Hello and frames over curl.
- Not checked: the HTML client in a real browser.

Departures (builder's choices, for review):
- A small `SessionStore` trait, implemented for ActorSystem with public operations only (get_actor, send, changes), so tests can make store reads and sends fail or stall without a fault hook.
- Each socket remembers what it already sent, so frames repeat only on reconnect, not on every one-second read.
- A stream key reused for a new stream closes the socket as `StreamReset`.

Docs: owner fixes integrated (ADR 0045). Docs commit 147f05ae2d1395f5c360b5c8bf3e6b3bb76f609b, whole files taken with git checkout, no merge, folded into 6e69ae7: README.md, docs/README.md, docs/agents.md, docs/guide/core.md, docs/guide/design.md (new), docs/guarantees.md (G2 row). The build loop checked: the docs commit holds only these six paths; they are byte-equal on the branch to 147f05a and the docs branch; main did not touch them since the base. No prose left to write for this change. Please check the guide still describes the code (ADR 0045).

Breaking: none. Library API, guarantees, wire formats and schema unchanged.
