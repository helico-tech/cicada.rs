# Design

The actor actions (`inspector-actions`) and Send Message (`inspector-send-message`) act on actors. Operators also act on
messages: retry a failed one, cancel a pending one, deliver one now, and do it for many rows at once. This change adds
those to the message panes and the tables of Messages and Outcomes. It reuses the actor actions' rules: one action at a
time on the target it names, a context switch dropping a pending result, untrusted alert text, a fresh read after a
write. The calls are `cicada-client`'s, on the same routes as `cicada`.

## Decisions

1. **Buttons by the message's state.** The message panes are the Messages pane and the Outcomes pane, the latter also
   on a followed Caused by or Caused link. A pending message that is not running gets Cancel… and Deliver Now; a running
   one gets none (Cancel Turn is the actor pane's); a failed or cancelled one gets Retry…; a done or purged one gets
   none. The state is the one the pane last read; the server decides by its own, and its answer is shown.
2. **Retry asks first, with how often it was retried.** A retry sends a new message with a new id, `caused_by` the
   original, and may repeat external work; the server does not skip a message already retried when given its id. A
   failed or cancelled message's turn sent nothing, so the messages it caused are its retries, and the pane already
   reads them (Caused). The sheet says "Send <type> to <address> again as a new message? It may repeat external work."
   and "Retried before: N times, as last read" ("at least N" when Caused has more, "not read" when Caused has not
   answered or failed). Cancel is the default button: Return and Escape cancel; Retry must be clicked. A 202 shows
   "retried as <new id>"; `None` shows "message not found"; a 409 (not retryable, too deep) or a 404 for a gone actor
   shows an alert with the server's message.
3. **Cancel and Deliver Now.** Cancel… asks ("Cancel message <id> to <address>? It will not be handled."), Cancel the
   default button as above. Its outcome maps to local text: cancelled, "is being handled; not cancelled", "already
   finished", "message not found". Deliver Now does not ask (it only moves the message forward in its mailbox); its
   answer shows "due now", and `None` "message not found"; a 409 shows the alert.
4. **Several rows.** The Messages and Outcomes tables allow multiple selection. The pane follows the clicked row as
   today, and back and forward record only that row. With two or more rows selected, the page's footer shows Retry N…
   (Outcomes; rows of status failed or cancelled only, the others not counted) or Cancel N… (Messages; rows not shown as
   running). More than 1,000 such rows disables the button ("at most 1,000"). A refresh keeps the selected rows still
   in the list and drops the others; a new prefix, status or context clears the selection. The ids are taken when the
   button is pressed and not changed after.
5. **Bulk without sending twice.** Bulk Cancel asks ("Cancel N messages? Ones that are running or finished by then are
   skipped.") and sends the ids in one `bulk_cancel`; it shows "cancelled K of N". Bulk Retry first reads Caused (limit
   1) for each id, at most 8 at a time, under its own generation; an id with a child is dropped as already retried, one
   whose read fails is dropped too. The sheet then says "Retry K of N as new messages? M already retried and S not read
   are skipped. It may repeat external work."; with K = 0 it says nothing is left to retry and offers only Close. A
   confirmed sheet sends the K ids in one `bulk_retry` and shows "retried K' of K". A transport error or timeout on a
   bulk call shows "result unknown; read the page again before retrying" and is never sent again on its own. Between
   the check and the send another client can retry; the sheet says "as just read".
6. **One action at a time, results and refreshes.** As for the actor actions: from the press until the answer, the
   page's action buttons and bulk buttons are disabled; a single action's target is the message id with the pane's
   generation, checked again before sending and on confirming ("selection changed; nothing done"); a bulk action's
   target is its id list and the page's list generation, and a new prefix, status or context drops it. A context
   switch or closed window drops any pending result. Alerts use the actor actions' text rules (problem title and detail
   or "HTTP <status>", control characters and credentials removed, 500 characters per part). After a successful write
   the page asks for a soft refresh queued behind one already running, also with Live paused; while paging blocks it,
   a single action's pane is read again by a read started after the answer. The Overview's and sidebar's failed count
   follow from that refresh.
7. **Where it lives.** A `message_actions` module in the model (the state rules, targets and checks, the Caused checks
   for bulk retry, the sheet texts and counts, the result mapping) using the `actions` module's alert text and refresh
   request; multiple selection, buttons, sheets and alerts in the `appkit` module. Nothing in the tray or the library
   changes.

## Tests

Against an in-process server: the buttons by state (pending, running, failed, cancelled, done) on both panes and on a
followed link; retry answering 202 with the new id, `None`, a 409 for a done message and a 404 for a gone actor; the
retry sheet's count (none, N, "at least N", "not read"); cancel's four outcomes; deliver now moving a message forward
and `None`; bulk cancel's count with one row running by then; bulk retry skipping a row already retried, and one whose
Caused read failed, and the K = 0 sheet; more than 1,000 rows disabling the button; a timed-out bulk call not sent
again; selection kept and dropped across a soft refresh and cleared by a new prefix; one action at a time, a changed
target sending nothing, a context switch dropping a result; the fresh read after each write, also with Live paused.
The table selection, sheets and alerts get a manual check list in the tasks (⇧- and ⌘-click, Cancel the default, Return
and Escape cancel, Retry only by click, VoiceOver labels).

## Docs

The handoff gives the app guide's Inspector section the message actions, that a retry is a new message that may repeat
external work and that bulk retry skips retried rows; the changelog's 0.26.0 section names them.
