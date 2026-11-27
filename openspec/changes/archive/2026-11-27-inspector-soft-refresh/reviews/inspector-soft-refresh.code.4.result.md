Verdict: pass (ready to land)

## Blocking

None. Round 3's finding is fixed.

## Answers

1. The sort keys match the server in both backends. SQLite and Postgres both use `ORDER BY actor_address, deliver_at, seq` and `finished_at, message_id`. Addresses and ids are BINARY or `COLLATE "C"`, which is byte order, the same as Rust's `String` and the derived `Ord` of `ActorAddress` and `MessageId`. Both backends store microseconds. `format_instant` writes 3, 6 or 9 digits without loss. `MessageView.to`, `deliver_at`, `seq` and `OutcomeView.finished_at` are copied straight from the record. `seq` is store-wide (UNIQUE in SQLite, `nextval` in Postgres), so the client's `id` tie-breaker is never needed.
2. Nothing is lost or shown twice. Every row at or before the boundary is in the fresh range. Old rows after it are kept and deduplicated by identity. A row that moved past the boundary drops out, and paging from the new cursor brings it back, because `seen` is rebuilt from the rows.
3. `read_range`, `replace_range`, `kept` and the stale mark are correct.
4. Both unit tests hold. The bulk test's `m140` assert is trivially true; `len == 450` is what proves the case.

## Notes

1. `tasks.md:8` and the test name `messages_refresh_stops_on_the_old_last_id…` still say "old last id".
2. `pager.rs:277` takes `old.last()`, not the old rows' greatest sort. After a refresh that kept a tail and then paged on, the list is not in server order. The next range can then be shorter, and rows past it are only marked stale, not lost.

## Not checked

I ran the `cicada-inspector` tests: 98 passed, 1 doctest passed. Not run: the workspace, clippy, fmt, the gate, AppKit, a real Postgres server.
