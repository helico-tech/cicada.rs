Verdict: ready for tasks

Both round-2 points are fixed in design §4 and in the tests paragraph. A Caused page with no rows but a next cursor is followed at once, and "None" shows only when the list ends with no rows. A failed linked read shows its error with Retry on the same id, and "not found" is kept for a missing message.

Notes (not blocking):
- Tasks should test the empty-page follow with more than one empty page in a row.
- Tasks should test that Retry after a failed linked read works from a Back/forward place.

Not checked: build, UI, strict OpenSpec validation.
