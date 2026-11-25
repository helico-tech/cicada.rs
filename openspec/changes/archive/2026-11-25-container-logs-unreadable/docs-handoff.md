# Docs handoff for container-logs-unreadable

Version: 0.22.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`logs` and `logs_since` of a container whose log the runtime can never read fail with `ErrorKind::Unsupported`, no
longer `Other`. A follower can now tell "stop, there will never be logs" from a passing daemon error, which also gives
`Other` (Factory issue 0021).

- `DockerRuntime`: Docker answers a log read of a container whose logging driver cannot be read (log driver `none`)
  with HTTP 501, "configured logging driver does not support reading". Both log calls map that 501 to `Unsupported`,
  with the message `logs of <name>: <Docker's message>`. Every other error keeps its kind (404 is `Conflict`, a lost
  connection `DaemonUnavailable`, anything else `Other`). The answer is the same for a created container and one that
  ran.
- `ErrorKind::Unsupported` now means "the runtime does not implement the call, or cannot do it for this container". A
  retry cannot help in both.
- The container actor: when the log read for an end is `Unsupported`, the end is recorded in that turn with the logs
  `(no logs: unsupported: logs of <name>: …)`. The error is not counted and the read is not retried. The rest of the
  end, such as the removal of a goal `Remove`, goes on as before. Other kinds still count and retry, and end with
  `(no logs: …)` only after `max_errors` in a row.
- The fake needs nothing new: `fail_always(Method::Logs, ErrorKind::Unsupported)` (or `Method::LogsSince`) scripts
  such a container.
- No public item is added or removed. No contract, wire format, schema or guarantee changes.

Proved by `a_log_read_answered_with_501_is_unsupported_and_a_500_stays_other` in
`crates/cicada-containers/src/docker/map.rs`; `logs_the_runtime_cannot_read_end_at_once_without_logs`,
`an_unreadable_log_does_not_hold_up_a_removal` and `other_from_a_log_read_is_still_counted_and_retried` in
`crates/cicada-containers/tests/actor.rs`; and the E2E step `a_log_driver_none_is_unsupported_created_and_after_it_ran`
in `e2e/tests/suite/container_docker.rs` (a container made by `docker create --log-driver none`).

## 1. The error table

**Where.** `docs/guide/containers.md`, `Errors`, the `Unsupported` row.
**Text.** `Unsupported` | The runtime does not implement this call, or cannot do it for this container, such as the
logs of a container whose log driver keeps none. A retry cannot help.
**Caveats.** None.

## 2. A follower stops at `Unsupported`

**Where.** `docs/guide/containers.md`, `Follow a log`, the paragraph after the numbered steps.
**Text.** Replace "and a log Docker cannot read (log driver `none`) is `Other`" with: a log the runtime can never read
(Docker's log driver `none`) is `Unsupported`: stop following then, as no retry will read it. `logs` gives the same
kind. Retry the other kinds, counting errors in a row.
**Caveats.** Before 0.22.0 this case was `Other`; a follower that matched `Other` to stop must match `Unsupported` now.

## 3. The actor's end without logs

**Where.** `docs/guide/containers.md`, `The end`, the paragraph "Runtime errors never fail a turn".
**Text.** Add: a log read the runtime answers with `Unsupported` does not count: the end is recorded at once with the
logs `(no logs: …)` and the runtime's message.
**Caveats.** None.

## 4. Upgrading to 0.22.0

**Where.** `docs/how-to/upgrading.md`, the 0.22.0 section.
**Text.** `cicada-containers`: `logs` and `logs_since` of a container whose log cannot be read (Docker's log driver
`none`) fail with `Unsupported` instead of `Other`. Code that matched `Other` for this case matches `Unsupported`.
**Caveats.** Not breaking at compile time; only the kind of this one error changes.

## Blocking

None.

## Status

Ran `cargo test -p cicada-containers --all-features`, the E2E class `ContainerDockerTest` against Docker 29.4.0, and
`cargo test -p cicada-examples --test docs`. All text above is proposed; no protected file was edited.
