# Design

Operators want to send a message to an actor from the inspector, as `cicada send` does (owner's proposal, split from
`inspector-actions`). The server checks a payload against the registered schema of its message type and refuses a
mismatch with 422 (`ActorClient::send_message`); the inspector checks it locally too, so the operator sees the problem
before sending.

## Decisions

1. **The sheet.** Send Message… in the actor pane (next to `inspector-actions`' buttons, with the same
   one-action-at-a-time rule and target checks) opens a sheet: a pop-up of the message types of the actor's type, read
   from `/types` (the Types page's cache when loaded, else one read; a failed read shows its error with Retry; an actor
   whose type is not in the answer shows "type not registered"), the chosen type's schema shown read-only beside the
   editor, and a plain-text JSON editor for the payload, prefilled with `{}`, with smart quotes, dash substitution and
   text replacement off. The sheet has no default button, so Return never closes it (it inserts a newline in the editor
   and does nothing on the pop-up); ⌘Return sends, and Escape cancels, also from the editor (its cancel action is bound
   to the sheet's Cancel, not to completion).
2. **The local check, by the server's settings.** The check copies the server's validator settings from `cicada-axum`'s
   `schema_check.rs`, with a comment naming it: `jsonschema` (the workspace's version) on draft 2020-12 whatever
   `$schema` says, no `format` assertion, and a retriever that loads nothing from outside the schema. A schema that
   cannot be compiled shows a note ("the server does not check this type's payloads") and leaves Send on, as the server
   lets such payloads through. A test sends the same payloads to an in-process server and checks that the local verdict
   agrees with the server's 202 or 422.
3. **Off the main thread, bounded.** The chosen type's schema is compiled once when it is chosen. An edit disables
   Send at once; the payload is checked 300 ms after the last edit, on a thread with a 64 MiB stack (as the server
   does), one check at a time; a result for an older payload or type is dropped. The check refuses a payload over 1 MiB
   of text, one that does not parse, and one nested deeper than the server's limit of 64 levels
   (`cicada::runtime::limits::MAX_DEPTH`, counted as the server does, ADR 0029) before any schema check, as the server
   does. Then it asks only `is_valid`, which stops at the first mismatch: `validate` cannot be stopped and can collect
   whole branches of errors, so the sheet shows "does not match the schema" with the schema beside it, not a pointer.
   Send is enabled only when the payload passes (or the schema could not be compiled). The server's check stays the
   authority: its 422 or 400 shows in the sheet with the server's text (which names the pointer), cleaned and cut as
   `inspector-actions` does, and the sheet stays open with the payload.
4. **One message id per payload.** The sheet makes a message id when it opens and makes a fresh one when the message
   type or the parsed payload changes (an edit that leaves the parsed value equal keeps it), so pressing Send again
   after a timeout re-sends the same id and the server does not deliver it twice; an edited payload after a lost answer
   goes as a new message. The server ignores the payload when it already has the id for the same target and type, and
   answers 200 within the message's retention. `send_message` with that id and default options (the client's retries on
   transport errors and 503). A 202 closes the sheet and shows "sent <id>"; a 200 closes it and shows "already sent
   <id>"; both ask for a fresh read (a new soft refresh, as `inspector-actions` does after a write), so the pane is read
   again. A 404 (`no-such-instance`) shows "actor not found" in the sheet; a 413 (too large for the server) and any
   other refusal show an alert, and the sheet stays open with the payload. A context switch or a closed window drops the
   result.
5. **A type without messages** shows "this type takes no messages" and no Send.
6. **Where it lives.** A `send` module in the model (the types for the pop-up, the compiled schema and its bounded check
   thread, the message id, the send and its result mapping) with tests against an in-process server; the sheet in the
   `appkit` module. No public API changes outside the inspector.

## Tests

Against an in-process server: the pop-up's types from `/types` and from the Types cache, a failed `/types` read with
Retry, an actor whose type is missing; the local verdict agreeing with the server's for the same payloads (draft 2020-12
whatever `$schema`, a `format` not asserted, a schema that cannot be compiled leaving Send on); a payload that does not
parse, one nested 65 levels refused before the schema, one that fails showing "does not match the schema"; Send disabled
at once on an edit; one check at a time with stale results dropped; the parity test on a type no actor hosts; the same
id re-sent after a timeout and the server's 200 shown as "already sent"; an edited payload after a lost answer sent with
a new id; a 202 with "sent" and the fresh read; the server's 422 shown in the sheet with the payload kept; 404 as "actor
not found"; 413 as an alert with the sheet kept; one action at a time with `inspector-actions`; the 1 MiB limit; a type
with no messages. The sheet gets a manual check list in the tasks (⌘Return sends, Escape cancels also from the editor,
Return never closes the sheet, smart quotes off, VoiceOver).

## Docs

The handoff gives the app guide's Inspector section Send Message and its local check; the changelog's 0.26.0 section
names it.
