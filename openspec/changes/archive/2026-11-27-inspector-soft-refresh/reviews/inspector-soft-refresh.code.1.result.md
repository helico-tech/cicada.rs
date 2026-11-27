Verdict: fix needed

## Blocking

1. **Rows lost at the seam on Messages (and Outcomes)**: `crates/cicada-inspector/src/pager.rs:387`. When the old last id is not reached, `replace_range` keeps the old tail by position: `old.skip(read)`. Pending messages are sorted by `actor_address, deliver_at, seq` (store `list_pending`). So a new message for an actor that sorts early pushes every row back one place.
   Scenario: 450 rows are loaded and 5 new messages arrive for `a/1`. The read returns 5 new rows plus old rows 0..295, and the tail kept is old rows 300..450. Old rows 295..299 are in neither part, so they vanish while still pending. Paging on brings them back later, but at the end, after the tail.
   The test `messages_refresh_to_three_pages_when_the_old_last_message_was_consumed` only removes rows at the front, never adds them.
   Fixed: keep the old rows after the last old row that the new range holds (or keep every old row the range does not hold), plus a test that inserts rows ahead of the seam.

## Notes

1. A manual Refresh restores the old scroll offset by clamping at the bottom, and `check_more` then pages again and again until it gets back to that depth. After ⌘R deep in a list, this reads many pages. It is bounded, but worth a manual check.
2. `cancel_refresh` drops a deferred "more" when the page changes or the Watches filter changes. The user has to scroll again.
3. The pane re-read (an Outcomes link, a node) keeps the refresh "running", so ordinary paging can wait up to 10 s.
4. When a history row (`pending`) arrives through a soft refresh, the new row replaces the turn or outcome "as read". `land_page` keeps it as read instead.
5. The docs handoff fits ADR 0045. The CHANGELOG line is there, and no protected docs are edited.

## Not checked

- I ran `cargo test -p cicada-inspector`: 94 passed.
- Not run: the workspace, clippy, fmt, the gate, E2E.
- AppKit scroll behaviour was not run at all. I read the code only: no crash path on an empty table or an index past the end (`rowForItem` -1 maps to None; indexes come from the new list).
- Keyboard and VoiceOver were not checked.
