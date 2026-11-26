Verdict: ready for tasks

## Blocking findings

None.

## Verified in the code

Decision 1 holds. A turn cut off at `max_turn` is `handler_failed(.., retryable: true)` (runtime/executor.rs:702-705). `failure_decision` turns that into `Rescheduled` while the poll's policy (7 attempts, actor/mod.rs:462) allows another run (completion.rs:197-208). Both stores then run `failures = failures + 1` on the same message (postgres/store.rs:2354, sqlite/store.rs:1252). `turn.message()` returns that envelope, `failures` included (turn.rs:393-398). The fourth run sees `failures == 3`.

## Notes

1. `failures` counts every thrown failure of the poll, not only timeouts: a store error in `deliver` or `progress` (actor/mod.rs:668, 964) adds to it too. The fixed text "ran past the turn limit 3 times" can be wrong. Use a neutral text with the message's `last_error`, e.g. `(no outputs: the collection turn failed 3 times: <last_error>)`.
2. `finish` calls `progress`, which is a `copy_out` too (actor/mod.rs:946). Say in design.md that the give-up path calls `record_pending` and skips the progress read. The spec's "no copy" already implies this.
3. `max_read` counts "bytes of the archive". The fake has no archive, and the collector promises that both runtimes keep and skip the same files (copy.rs:1-2). Define it as content bytes of the entries (header sizes), counted the same way in both.
4. Behaviour change for existing jobs: a big skipped file before small ones now drops the small ones, where today they are kept. Name it in the CHANGELOG and the handoff. Also update the `MAX_TURN` rustdoc (actor/mod.rs:106-109, "waits for the next Check").
5. Decision 3 adds "while collecting", which the proposal does not ask for. At `max_errors` the collection already ends with the error in its report. Drop it or say why.

## Questions for the owner

- Q1: Is 4 × `max_bytes` the default Factory wants? Pick: yes.

## Not checked

The Docker archive reader's stop point (docker/archive.rs), the lost-claim path, the proposals after `placement-offers`, and the docs blocks.
