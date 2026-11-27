# Design

The inspector reads only. Operators also want to act on what they see (owner's proposal): pause an actor, cancel a
stuck turn, purge or delete. This change gives the actor pane's actions; sending a message (`inspector-send-message`)
and the message and Outcomes actions (`inspector-message-actions`) follow (all of it did not fit five tasks). The
actions use `cicada-client`'s calls, the same routes as `cicada`, so the server's middleware and tokens apply.

## Decisions

1. **The actor pane's buttons.** Pause or Resume (by the actor's paused flag; `pause_mailbox`, `resume_mailbox`),
   Cancel Turn (shown while the pane shows a running turn; `cancel_turn`), Purge Mailbox… (`purge_mailbox`) and
   Delete… (`delete_actor`, which removes the actor and every descendant and purges their mailboxes). A button is
   disabled while its call runs; one action at a time per pane.
2. **Asking first.** Purge Mailbox… and Delete… open a sheet that names what goes: the actor's address, the number of
   pending messages as the pane last read them ("about N"), and for Delete "and every descendant" with the number of
   loaded children when known ("at least N"). The confirm button is the destructive style and not the default, so
   Return cancels; Esc cancels. Pause, Resume and Cancel Turn act at once, as `cicada` does.
3. **Results and refusals.** On success the pane shows a short notice (purged N messages, deleted, paused, …) and the
   page runs a soft refresh (from `inspector-soft-refresh-actors`), so the row and the pane follow; a deleted actor's
   row disappears and the pane says "not found". A refusal of the API (409, 404, 422) shows an alert with the server's
   problem title and detail; a transport error shows its text; neither changes the pane. Credentials never appear.
4. **Stale panes.** An action carries the pane's generation: when the selection changed while it ran, its notice is not
   shown (the alert of a refusal still is, naming the actor), and the soft refresh still runs. Cancel Turn whose turn
   has already ended answers false; the pane says the turn had ended.
5. **Where it lives.** An `actions` module in the model (the calls, the confirmation texts, the result and refusal
   mapping, the generation check) with tests against an in-process server; the buttons, sheets and alerts in the
   `appkit` module. Nothing in the tray or the library changes.

## Tests

Against an in-process server: pause and resume flip the flag; cancel turn on a running turn and on an ended one; purge
reports its count; delete removes the actor and its children; each refused call (an unknown actor, a 409) mapped to an
alert with the server's text and no pane change; one action at a time; a stale action's notice dropped and the soft
refresh still run; the confirmation texts with their counts. The buttons, sheets and alerts get a manual check list in
the tasks (destructive button not default, Return and Esc cancel, VoiceOver labels).

## Docs

The handoff gives the app guide's Inspector section the actor actions, the confirmation for purge and delete, and that
the actions go through the same routes as `cicada`; the changelog's 0.26.0 section names them.
