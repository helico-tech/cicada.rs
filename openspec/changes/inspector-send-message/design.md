# Design

Operators want to send a message to an actor from the inspector, as `cicada send` does (owner's proposal, split from
`inspector-actions`). The server checks a payload against the registered schema of its message type and refuses a
mismatch with 422 (`ActorClient::send_message`); the inspector checks it locally too, so the operator sees the problem
before sending.

## Decisions

1. **The sheet.** Send Message… in the actor pane (next to `inspector-actions`' buttons, with the same one-action-at-a-
   time rule and target checks) opens a sheet: a pop-up of the message types of the actor's type, read from `/types`
   (the Types page's cache when loaded, else one read), the chosen type's schema shown read-only beside the editor, and
   a plain-text JSON editor for the payload, prefilled with `{}`. Send and Cancel; Cancel is the default (Return inserts
   a newline in the editor; ⌘Return sends; Escape cancels).
2. **The local check.** As the operator types (debounced 300 ms), the payload is parsed as JSON and checked against the
   chosen message type's schema with `jsonschema`, the crate and version the server uses (from the workspace); the first
   few errors show under the editor with their JSON pointer. Send is enabled only when the payload parses and passes.
   The server's check stays the authority: its 422 or 400 shows in the sheet with the server's text (cleaned and cut as
   `inspector-actions` does) and the sheet stays open with the payload.
3. **Sending.** `send_message(address, type, payload, options)` with default options (a new message id, the client's
   retries on transport errors and 503). Success closes the sheet, shows "sent <id>" in the pane, and asks for a fresh
   read (a new soft refresh, as `inspector-actions` does after a write), so the message shows under Mailbox. A 404
   (`no-such-instance`) shows "actor not found"; a refusal other than 400, 404 or 422 shows an alert. A context switch
   or a closed window drops the result.
4. **Limits.** The payload editor accepts at most 1 MiB of text; past that Send is disabled with a note. A type without
   message types shows "this type takes no messages" and no Send.
5. **Where it lives.** A `send` module in the model (the types for the pop-up, the local check, the send and its result
   mapping) with tests against an in-process server; the sheet in the `appkit` module.

## Tests

Against an in-process server: the pop-up's types from `/types` and from the Types cache; a payload that parses and
passes enabling Send; one that does not parse, and one that fails the schema with its pointer; a success with "sent" and
the fresh read; the server's 422 shown in the sheet with the payload kept (with the local check bypassed in the test);
404 as "actor not found"; one action at a time with `inspector-actions`; the 1 MiB limit; a type with no messages. The
sheet gets a manual check list in the tasks (⌘Return, Escape, Return in the editor, VoiceOver).

## Docs

The handoff gives the app guide's Inspector section Send Message and its local check; the changelog's 0.26.0 section
names it.
