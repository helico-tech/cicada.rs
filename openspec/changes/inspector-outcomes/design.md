# Design

The Outcomes item of the sidebar still says "Coming soon". Operators look there for what failed and why (owner's
proposal; mockup page Inspector). This change gives the Outcomes page, its pane with the cause chain, and the failed
count in the sidebar. Running Turns and Nodes follow in `inspector-nodes` (all of it did not fit five tasks).

## Decisions

1. **The Outcomes list.** `outcomes` through the pager of `inspector-messages` (100 a time, the server's cursor passed
   back unchanged), under the search prefix and a status switch in the toolbar: Failed (the default), Cancelled, Done,
   All. Rows: message type, actor, attempt, finished time, and the error's first line. Pages are not a snapshot, so the
   first row per message id within a generation is kept, as on Messages. Changing the switch starts a new generation.
2. **The failed count in the sidebar.** The Outcomes item shows `failed` from `/stats` with no prefix, read with the
   Overview's refresh; "—" when the server sends none (an older server), as on the Overview. It is the count of failed
   outcomes the listing would show, labelled "kept by retention" in its tooltip.
3. **The pane, from the row.** An outcome row carries its message, so the pane needs no read for it: the status and
   error, attempts, failures and the last retried error, the retry policy, the envelope (to, from, correlation id,
   headers, finished), and the payload formatted on selection with the Actors pane's formatter, its cut and its run cap,
   a stale formatting dropped. Show Actor jumps to its actor, as on Messages.
4. **The cause chain, one step at a time.** "Caused by" shows the message's `caused_by` id as a link: following it reads
   `status(id)` and shows that message in the pane (whether pending, running or finished), recorded in back and forward
   as a place; "not found" when it is past retention. "Caused" lists the messages its turn sent (`caused_by`, its first
   50, with "more…" by cursor), each a link that does the same. Reads carry the pane's generation and are dropped when
   it changes.
5. **Read only, and where it lives.** No retry or cancel until `inspector-actions`. Model code in `outcomes.rs` with
   tests against an in-process server; the `appkit` module gains the page (an `NSTableView` sharing the search field,
   footer and pane toggle), the status switch and the links.

## Tests

The list by status and prefix, paging by the unchanged cursor, a duplicate id kept once, the switch starting a new
generation; the sidebar count and its "—"; the pane from the row with the payload formatted on selection; Caused by
followed to a pending and a finished message and to one past retention; Caused listed with "more…"; the chain recorded
in back and forward; a stale chain read dropped. The window gets a manual check list in the tasks. Tests keep a
wall-clock bound and do not depend on machine speed.

## Docs

The handoff gives the app guide's Inspector section the Outcomes page, the cause chain and the sidebar count; the
changelog's 0.26.0 section names them.
