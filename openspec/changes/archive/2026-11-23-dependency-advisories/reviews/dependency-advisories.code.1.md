---
id: dependency-advisories.code.1
kind: code
change: dependency-advisories
round: 1
base: d75847f3177f7134e65e943cc54aaa8d76e77b24
head: 5c0ade01e83d25704eb5b092030de6099fb546f5
requested: 2026-11-23T14:52:44+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done dependency-advisories.code.1 <file>`.

## Note from the requester

Version: 0.20.6

Builder's report (summary): head 5c0ade0. Tasks 1-4 done. `libsqlite3-sys = ">=0.39"` (lockfile 0.40.0), `sqlx` 0.9.1;
`deny.toml` accepts only RUSTSEC-2026-0543; gate check `advisories` in `full`; ci.yml installs cargo-deny 0.20.2.
schemars floor raised to 1.2.3: `CICADA_UPDATE_CONTRACTS` rewrote `sample-service/tests/contract-texts.json` (17 text
hashes) and `docs/openapi.json` changed in key order only; the 29 sample contract texts are equal as parsed JSON;
`docs/reference/contracts/`, `fixtures/` and contract hashes unchanged. Full gate passed on 5c0ade0 (one earlier run
on the same head failed once in a known flaky sample-service test, unrelated: a line read before its newline). E2E
not run; the release runs it.
