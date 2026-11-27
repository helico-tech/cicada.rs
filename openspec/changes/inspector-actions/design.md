# Design

The inspector reads only. Operators also want to act on what they see (owner's proposal): pause an actor, cancel a
stuck turn, purge or delete. This change gives the actor pane's actions; sending a message (`inspector-send-message`)
and the message and Outcomes actions (`inspector-message-actions`) follow (all of it did not fit five tasks). The
actions use `cicada-client`'s calls, the same routes as `cicada`, so the server's middleware and tokens apply.

## Decisions

1. **The actor pane's buttons.** Pause or Resume (by the actor's paused flag; `pause_mailbox`, `resume_mailbox`), Cancel
   Turn… (shown while the pane shows a running turn; `cancel_turn`), Purge Mailbox… (`purge_mailbox`) and Delete…
   (`delete_actor`, which removes the actor and every descendant and purges their mailboxes). Writes go by address, so
   they act on whatever actor holds that address when the server gets them; the sheets name the address.
2. **One action at a time, on the target it names.** An action's target is the selected actor's address and the pane's
   generation when its button is pressed. From that moment (a sheet opening) until its request ends, the pane's action
   buttons are disabled, also when the user selects another actor meanwhile. Before sending, and again when a sheet is
   confirmed, the model checks that the target is still the selected actor in the same context; if not, nothing is sent
   and the pane says "selection changed; nothing done". A context switch or a closed window drops any pending action's
   result: no alert, no notice, no refresh.
3. **Cancel Turn asks too.** The server cancels the turn it finds running for the actor when the request arrives, which
   may be a newer turn than the one shown. So Cancel Turn… opens a sheet: "Cancel the turn running for <address>
   (message <id> as last read)? The server cancels whichever turn is running when it receives this." An answer of true
   means the cancel was recorded ("cancel requested", nothing more, as the handler may already have finished); false
   means no turn was running ("no turn was running").
4. **Asking first, with honest counts.** Purge Mailbox… names the actor's address and its pending messages as the pane
   last read them (the pending rows among the mailbox's first 50 rows, the running one not counted: "N pending, as last
   read", or "at least N pending, as last read" when that page had a next cursor, or "not read" when the mailbox read
   failed or has not answered), and says the running message, if any, is not purged. Delete… names the address, says it
   removes "this actor and every descendant, with all their mailboxes", and gives the loaded children ("at least N
   children" when the list was paged, "children not read" when the row was never opened). In every sheet Cancel is the
   default button: Return and Escape both cancel; the destructive button has no key equivalent and must be clicked.
5. **Results, absence and refusals.** The client turns an expected 404 into false or `None`; the inspector maps it to
   local text: "actor not found" for pause, resume, purge and delete, "no turn was running" for cancel turn. Any other
   error shows an alert. Its text is untrusted: the problem's title and detail when the body is a problem, else only
   "HTTP <status>", or the transport error's text; control characters are removed, credentials removed, and each part
   cut at 500 characters; and leaves the pane as it was. A success shows a short notice (purged N messages, deleted,
   paused, resumed, cancel requested). Credentials never appear.
6. **A fresh read after every write.** After a successful write the page asks for a soft refresh (from
   `inspector-soft-refresh-actors`), queued as a new refresh behind one already running (never merged into it, as that
   one's reads began before the write), so every pane read of the post-write refresh starts after the write's answer; it
   is independent of live updates, so it runs also with Live paused (pausing Live cancels only automatic refreshes,
   never this one). When the soft refresh cannot run (a page in flight waits; failed paging skips it), the pane is read
   again on its own, by a read started after the write's answer, never by a read that began before it. Delete goes
   further: when it answers, every cached occurrence of the actor and its known descendants is removed from the outline
   at once, inside or outside the refresh's bounds, and every root or children page read that began before the answer is
   fenced so it cannot bring them back. The pane says "not found" only when it still shows the deleted actor; when the
   user selected another actor meanwhile, that pane stays as it is and the Delete notice is not shown (the result is
   fenced by the pane's selection generation), though the outline is still pruned.
7. **Where it lives.** An `actions` module in the model (the calls, the targets and their checks, the sheet texts and
   counts, the result, absence and refusal mapping, the refresh request) with tests against an in-process server; the
   buttons, sheets and alerts in the `appkit` module. Nothing in the tray or the library changes.

## Tests

Against an in-process server: pause and resume flip the flag; cancel turn answering true and false; purge reports its
count and leaves the running message; delete removes the actor and its children; each 404 mapped to its local text;
another error mapped to an alert with the server's text and no pane change; one action at a time from sheet to answer,
also across a reselection; a target that changed before sending or confirming sends nothing; a context switch dropping a
pending result; the fresh read after a write, also with Live paused and while paging blocks the soft refresh; a deleted
actor's cached occurrences and known descendants removed at once and a page that began before the answer not bringing
them back; a Delete answering after another actor was selected leaving that pane and showing no notice; the paging
fallback reading the pane only after the write's answer; the sheet texts with their counts (a full first page with one
running message counting 49 pending, "at least" from the cursor, "not read"); alert text without credentials or control
characters, cut at 500 characters, and "HTTP <status>" for a body that is not a problem; a write while a soft refresh
runs queuing a new one whose pane read starts after the answer. The buttons, sheets and alerts get a manual check list
in the tasks (Cancel the default, Return and Escape cancel, the destructive button only by click, VoiceOver labels).

## Docs

The handoff gives the app guide's Inspector section the actor actions, the confirmation for purge and delete, and that
the actions go through the same routes as `cicada`; the changelog's 0.26.0 section names them.
