# Design

The Outcomes item of the sidebar still says "Coming soon". Operators look there for what failed and why (owner's
proposal; mockup page Inspector). This change gives the Outcomes page, its pane with the cause chain, and the failed
count in the sidebar. Running Turns and Nodes follow in `inspector-nodes` (all of it did not fit five tasks).

## Decisions

1. **The Outcomes list.** `outcomes` through the pager of `inspector-messages` (100 a time, the server's cursor passed
   back unchanged), under the search prefix and a status switch in the toolbar: Failed (the default), Cancelled, Done,
   All. Rows: message type, actor, attempt, finished time, the error's first line, and on All a status column (failed,
   cancelled, done, purged). Pages are not a snapshot: within a generation the first row per message id is kept, and
   paging goes on after a page of only duplicates, as on Messages. Changing the switch starts a new generation that
   resets together the cursor, the seen ids, a pending search, the selection and the pane's work.
2. **The failed count in the sidebar.** The Outcomes item shows `failed` from `/stats` with no prefix: the store-wide
   number of failed outcomes the listing would show at the time of that read. It is read with the Overview's refresh and
   with Outcomes' Refresh. It shows "…" until the first read answers and again after a context switch (which clears it),
   keeps its last value with the error in its tooltip when a read fails, and shows "—" when the server sends no `failed`
   (an older server). Its tooltip says "failed, kept by retention, as of the last read".
3. **The pane, from the row.** An outcome row carries its message, so the pane needs no read for it: the status and
   error, attempts, failures and the last retried error, the retry policy, the envelope (to, from, correlation id,
   headers, finished), and the payload formatted on selection with the Actors pane's formatter, its cut and its run cap,
   a stale formatting dropped. Show Actor jumps to its actor, as on Messages.
4. **The cause chain, as places.** "Caused by" shows the message's `caused_by` id as a link; "Caused" lists the messages
   caused by this one (those its turn sent and operator retries of it; `caused_by`, its first 50, with "more…" by
   cursor). Following any link is a new place in back and forward, holding the message id; it reads `status(id)` fresh
   and shows that message in the pane, whether pending, running or finished, or "not found" when it is past retention; a
   read that fails shows its error with Retry, which reads the same id again. An outcome's place also keeps the outcome
   as read, so Back restores its pane even when its row is no longer loaded (another status, a later page, a new prefix)
   or has expired since; its row is selected only when it is loaded. "Caused" keeps one request in flight per pane,
   Retry asks the same cursor again, a page with no rows but a next cursor is followed at once, "None" shows only when
   the list ends with no rows, and a gone parent says "not found". Reads carry the pane's generation and are dropped
   when it changes. A context switch clears the history and cancels pending reads, as on Messages.
5. **Read only, and where it lives.** No retry or cancel until `inspector-actions`. Model code in `outcomes.rs` with
   tests against an in-process server; the `appkit` module gains the page (an `NSTableView` sharing the search field,
   footer and pane toggle), the status switch and the links.

## Tests

The list by status and prefix, paging by the unchanged cursor, a duplicate id kept once and paging past a page of only
duplicates, the switch resetting the cursor, seen ids, search, selection and pane work, the status column on All; the
sidebar count's "…", its kept value on a failed read, "—", and its reset on a context switch; the pane from the row with
the payload formatted on selection; Caused by followed to a pending and a finished message and to one past retention;
Caused with "more…", one request at a time, Retry on the same cursor, an empty page with a next cursor followed, "None"
and "not found"; a failed linked read with Retry, apart from "not found"; each link a place, Back restoring an outcome
whose row is not loaded; a context switch clearing the history; a stale chain read dropped. The window gets a manual
check list in the tasks. Tests keep a wall-clock bound and do not depend on machine speed.

## Docs

The handoff gives the app guide's Inspector section the Outcomes page, the cause chain and the sidebar count; the
changelog's 0.26.0 section names them.
