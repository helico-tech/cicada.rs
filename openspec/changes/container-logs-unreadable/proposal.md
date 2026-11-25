# Proposal

## Why

With Docker's log driver `none`, `logs` and `logs_since` fail with `Other`, which is also what a passing daemon error
gives. A follower cannot tell "stop, there will never be logs" from "try again", so Factory's run stays in Collecting
(Factory issue 0021, request from the Factory session, 2026-11-25). The rustdoc of `logs_since` meant `Other` as
"stop retrying", but `ErrorKind` also says a caller may retry the others, counting errors in a row.

## What Changes

- A distinct error kind for a container whose log can never be read, on Docker (the fake scripts it with its
  existing failure calls). Options for the
  design: `Unsupported` with a clear message (Factory's suggestion; today it means "the runtime lacks the call"), or a
  new `ErrorKind` variant (the enum is `#[non_exhaustive]`).

To decide in the design: the kind; whether it is a change of behaviour that a patch may carry (a caller that matched
`Other` for this case sees another kind); how Docker's answer is recognised.

## Out of scope

Reading logs from other drivers through other APIs. Factory's own Collecting state.

## Order

After `container-job-max-turn`, before `app-tasks`.

## Impact

`crates/cicada-containers` (error kinds, the Docker backend's log calls, the fake, the E2E class with a `none` driver).
