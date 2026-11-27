# Design

The actor actions (`inspector-actions`) and Send Message (`inspector-send-message`) act on actors. Operators also act on
messages: retry a failed one, cancel a pending one, deliver one now, and do it for many rows at once. This change adds
those to the message panes and the tables of Messages and Outcomes. It reuses the actor actions' rules: one action at a
time on the target it names, a context switch dropping a pending result, untrusted alert text, a fresh read after a
write. The calls are `cicada-client`'s, on the same routes as `cicada`.

## Decisions

1. **Buttons by what the pane knows.** The message panes are the Messages pane and the Outcomes pane, the latter also on
   a followed Caused by or Caused link. The Messages page lists pending messages from `GET /messages`, which does not
   say whether one is running, so its pane offers Cancel… and Deliver Now on every row and shows the server's answer. On
   the Outcomes pane, a followed link read by `status` knows the state: a pending message that is not running gets
   Cancel… and Deliver Now, a running one gets none (Cancel Turn is the actor pane's), a failed or cancelled one (also
   the selected outcome) gets Retry…, a done or purged one gets none. The server decides by its own state, and its
   answer is shown.
2. **Retry asks first, with the retries still kept.** A retry sends a new message with a new id, `caused_by` the
   original, and may repeat external work; the server does not skip a message already retried when given its id. A
   failed or cancelled turn commits no sends, so the messages a failed or cancelled message caused are its retries, and
   the pane already reads them (Caused). Retention can remove a retry that finished (it copies the original's
   retention), so the count is of retries still kept. The sheet says "Send <type> to <address> again as a new message?
   It may repeat external work." and "Retries still kept: N, as last read" ("at least N" when Caused has more, "not
   read" when Caused has not answered or failed), with "a retry removed by retention is not counted". Cancel is the
   default button: Return and Escape cancel; Retry must be clicked. A 202 shows "retried as <new id>"; `None` shows
   "message not found"; a 409 (not retryable, too deep) or a 404 for a gone actor shows an alert with the server's
   message.
3. **Cancel and Deliver Now.** Cancel… asks ("Cancel message <id> to <address>? It will not be handled."), Cancel the
   default button as above. Its outcome maps to local text: cancelled, "is being handled; not cancelled", "already
   finished", "message not found". Deliver Now does not ask (it only moves the message forward in its mailbox); its
   answer shows "due now", and `None` "message not found"; a 409 shows the alert.
4. **Several rows, held by the model.** The Messages and Outcomes tables allow multiple selection. Each model keeps a
   set of selected ids beside the pane's message (the clicked row, as today); back and forward record only that one.
   With two or more rows selected, the page's footer shows Retry N… (Outcomes; rows of status failed or cancelled, the
   others not counted) or Cancel N… (Messages; every selected row). More than 1,000 such rows disables the button ("at
   most 1,000"). A soft refresh keeps the selected ids still in the list and drops the others; a manual Refresh, a new
   prefix, status or context clears the set, as they clear the selection today. The ids are taken when the button is
   pressed and not changed after.
5. **Bulk, bounded and without sending twice.** Bulk calls go in chunks of 100 ids, one after another, each under the
   inspector's deadline, with a progress line ("N of M") and a Stop that sends no further chunk. Bulk Cancel asks
   ("Cancel N messages? Ones that are running or finished by then are skipped.") and shows "cancelled K of N". Bulk
   Retry first reads Caused (limit 1) for each id, at most 8 at a time, within 60 s overall, with a progress line and
   Stop; an id with a child (or an empty page with a next cursor) is skipped as already retried, one whose Caused
   answers "not found" as gone, one whose read fails or did not answer in time as not read. The sheet then says "Retry K
   of N as new messages? M already retried, G gone and S not read are skipped, as just read. It may repeat external
   work."; with K = 0 it says nothing is left to retry and offers only Close. A confirmed sheet sends the K ids and
   shows "retried K' of K". Stop during the checks ends the run with nothing sent; Stop during the writes shows
   "stopped: K done of N; the rest not sent". A chunk whose call fails or times out stops the run: the page says how
   many were done before it, that the result of that chunk is unknown and the rest were not sent, and to read the page
   again; nothing is sent again on its own. Between the check and the send another client can retry.
6. **One action at a time, results and refreshes.** As for the actor actions: from the press until the answer (for bulk,
   the last chunk), the page's action and bulk buttons are disabled; a single action's target is the message id with the
   pane's generation, checked again before sending and on confirming ("selection changed; nothing done"). A bulk
   action's ids are fixed at the press; confirming its sheet after a manual Refresh, a new prefix, status or context
   sends nothing ("selection changed; nothing done"). Once sent, a bulk result shows in the page's footer whatever the
   list does meanwhile, and only a context switch or a closed window drops it (and stops further chunks), as it drops
   any pending result. Alerts use the actor actions' text rules (problem title and detail or "HTTP <status>", control
   characters and credentials removed, 500 characters per part). After a successful write (for bulk, after the run) the
   page asks for a soft refresh queued behind one already running, also with Live paused; a single action's message is
   also read by `status`, started after the answer, and fills the pane unless the pane shows another message by then, so
   a row past the soft refresh's range, or a refresh that paging blocks, never leaves the pane showing the row as it was
   before the write. The Overview's and sidebar's failed count follow from that refresh.
7. **Where it lives.** A `message_actions` module in the model (the state rules, targets and checks, the selected-id
   sets' rules, the chunked bulk runs and the Caused checks, the sheet texts and counts, the result mapping) using the
   `actions` module's alert text and refresh request; the tables' multiple selection, buttons, sheets and alerts in the
   `appkit` module. Nothing in the tray or the library changes.

## Tests

Against an in-process server: the buttons on the Messages pane and by state (pending, running, failed, cancelled, done)
on the Outcomes pane and a followed link; Cancel on a Messages row that is running by then showing "is being handled;
not cancelled"; retry answering 202 with the new id, `None`, a 409 for a done message and a 404 for a gone actor; the
retry sheet's count (none, N, "at least N", "not read"); cancel's four outcomes; deliver now moving a message forward
and `None`; bulk cancel's count with one row running by then; bulk retry skipping a row already retried, one whose
Caused is not found and one whose read failed, and the K = 0 sheet; chunks of 100 with progress, Stop sending no further
chunk, a failed chunk stopping the run and never sent again; the Caused checks' overall limit; more than 1,000 rows
disabling the button; the selected-id set kept and dropped across a soft refresh and cleared by a manual Refresh and a
new prefix; a bulk sheet confirmed after a Refresh sending nothing; one action at a time, a changed target sending
nothing, a context switch dropping a result and stopping chunks; the fresh read after each write, also with Live paused,
and a cancelled Messages row past the refreshed range read again by `status`; Stop during the checks and during the
writes. The table selection, sheets and alerts get a manual check list in the tasks (⇧- and ⌘-click, Cancel the default,
Return and Escape cancel, Retry only by click, VoiceOver labels).

## Docs

The handoff gives the app guide's Inspector section the message actions, that a retry is a new message that may repeat
external work and that bulk retry skips rows with retries still kept; the changelog's 0.26.0 section names them.
