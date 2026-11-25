# Design

A container whose log can never be read (Docker's log driver `none`, set as the daemon's default) makes `logs` and
`logs_since` fail with `Other`, the kind of a passing daemon error too. A follower cannot tell "stop" from "try
again" (Factory issue 0021), and the container actor retries it like any runtime error before it ends with
`(no logs: …)`. Such a log gets the kind `Unsupported`, which already means "a retry cannot help".

## Decisions

1. **`Unsupported`, widened.** `logs` and `logs_since` fail with `Unsupported` for a container whose log the runtime
   cannot read. The doc of `ErrorKind::Unsupported` becomes "the runtime does not implement the call, or cannot do it
   for this container, such as the logs of a container whose log driver keeps none"; a retry cannot help in both.
   Not taken: a new `ErrorKind` variant (the enum is `#[non_exhaustive]`, so it would not break, but every host that
   matches the kinds learns one more for one cause; Factory asked for `Unsupported`).
2. **Docker's answer: the 501.** Docker answers a log read of a container whose driver cannot be read with HTTP 501
   ("configured logging driver does not support reading"). The log calls map a 501 to `Unsupported`, with Docker's
   message; every other error keeps its mapping. Once Docker streams, an error is text in the log, not an error, so
   only the status counts (design review, note 1). Not taken: checking the container's
   `HostConfig.LogConfig.Type` first (a list of drivers to keep up to date; dual logging makes most drivers readable,
   and the 501 is Docker's own verdict).
3. **The container actor stops reading at once.** Its logs step counts a failed read like any runtime error and ends
   with `(no logs: …)` only after `max_errors` in a row. An `Unsupported` read ends at once with
   `(no logs: <message>)`, without counting it or retrying the read; the rest of the end (such as the removal poll
   of a goal `Remove`) goes on as today. Other kinds keep the count.
4. **Named under Changed; the specs are modified.** A caller that matched `Other` for this case sees `Unsupported`.
   The rustdoc of `logs_since`, `DockerRuntime`'s rustdoc and the 0.21.0 changelog promised `Other`; the changelog
   names the old promise under Changed. The batch is 0.22.0 already (`watch-absent-target`), so the version question
   of the review is moot. The spec deltas modify the rules they change ("Errors say whether a retry can help",
   "Docker's errors map onto error kinds", "A deadline and errors in a row end a container"), so the archived specs
   do not contradict each other (design review, blocking).
5. **The fake needs nothing new.** A test scripts such a container with `fail_always(Method::LogsSince,
   ErrorKind::Unsupported)` (and `Method::Logs`). The rustdoc of the trait's log calls, of `ErrorKind::Unsupported` and
   of `DockerRuntime` (which promises `Other` today) says the kind.

## Tests

- Unit (`src/docker/map.rs`): a 501 of a log read maps to `Unsupported`; a 500 stays `Other`.
- Actor, with the fake: `Unsupported` from `logs` ends the container's end at once with `(no logs: …)`, `errors` not
  counted; `Other` keeps the retries.
- E2E (`e2e/tests/suite/container_docker.rs`): a container made with `docker create --log-driver none` (through the
  CLI helper, as `ContainerSpec` has no log driver) gives `Unsupported` from `logs` and `logs_since`, created and after
  it ran.

## Docs

Nothing breaks. The handoff gives the container page the kind, and that a follower stops at `Unsupported`.
