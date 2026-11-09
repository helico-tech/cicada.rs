---
status: todo
type: bug
created: 2026-11-09
---

# 0900 `connect_options` error can print a URL password

## Problem

`PostgresConnection::connect_options` puts `self.url` in its "Malformed PostgreSQL URL" error without `redact`. A
password passed as a query parameter (`postgres://h/db?password=…`) stays in `url`, so it can reach that message.
`parse` promises its messages never contain the password; this path does not keep that promise.

## How to reproduce

Call `connect_options` on a `PostgresConnection` whose URL has `?password=…` and that sqlx refuses to parse.

## Notes

Found while removing JDBC URLs (epic 0003). Use `redact(&self.url)` there.
