---
status: todo
type: idea
created: 2026-10-07
---

# 0501. `PostgresStoreConfig` has no map of extra connection properties

## Problem

Kotlin's `PostgresStoreConfig.properties` passes extra pgjdbc properties (`ssl`, `sslmode`, …) to every connection.
The Rust config has no equivalent: connection parameters go into the URL query (`sslmode`, `sslrootcert`,
`application_name`, `options`, …; see `PostgresConnection`), and TLS client certificates or other `PgConnectOptions`
settings cannot be set without a URL parameter for them.

Also different by necessity: `socket_timeout` bounds a whole transaction attempt (client side; the connection is then
closed), not every single read as pgjdbc's `socketTimeout` does, because sqlx has no per-read timeout.

## Notes

If needed, add `PostgresStore::connect_with(options: PgConnectOptions, config)` that takes sqlx options directly; the
store still adds `search_path` and the application names.
