# Docs handoff for inspector-send-message

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The macOS inspector's actor pane has a Send Message… button. It opens a sheet that sends one message to the selected
actor through `ActorClient::send_message`, the same route and credentials as `cicada send`. The payload is checked in
the inspector before Send is enabled, with the validator settings of the server's HTTP door; the server stays the
authority. No public API changed. The model and the server's answers are covered by
`crates/cicada-inspector/src/tests/send.rs` against an in-process server on virtual time; the native sheet has a manual
check list.

## 1. Send Message in the inspector

**Where.** `docs/guide/app.md`, `Inspector on macOS`, in the description of the Actors pane, after the actions.

**Text.** **Send Message…** opens a sheet for the selected actor. Its pop-up lists the message types of the actor's
type, from the Types page's list when it holds the type, otherwise from one read of `/types`. A failed read shows its
error with Retry; an actor whose type the server does not know shows "type not registered", and a type without
message types shows "this type takes no messages" with no Send. The chosen type's JSON Schema is shown read only beside
a plain-text JSON editor that starts with `{}`; smart quotes, dashes, text replacement and spelling correction are off.
Return adds a new line and never closes the sheet; ⌘Return sends and Escape cancels, also from the editor.

The payload is checked 300 ms after the last edit, and Send stays off until it passes. The check refuses a payload
over 1 MiB of text, one that does not parse, and one nested deeper than 64 levels, then checks it against the schema as
the server does: draft 2020-12 whatever `$schema` says, no `format` asserted, nothing loaded from outside the schema.
A mismatch shows "does not match the schema" with the schema beside it; no place in the payload is named. A schema the
validator cannot compile shows "the server does not check this type's payloads" and leaves Send on.

Send keeps one message id for the sheet until the message type or the parsed payload changes (reformatting the same
JSON keeps it). After a timeout, Send again sends the same id, and the server's answer that it already has the message
shows as "already sent <id>": it is delivered once. A payload edited after a lost answer goes as a new message. A
success closes the sheet, shows "sent <id>" in the pane and reads the pane again. The server's 400 or 422 text, and
"actor not found" for an actor deleted meanwhile, show in the sheet with the payload kept; any other refusal, such as a
413 for a body over the server's limit, shows in an alert over the sheet, which stays open. While the sheet is open no
other action runs. Switching context or closing the window drops a send's answer.

**Caveats.** The local check is a convenience, not the server's: the server also refuses a NUL character in the payload
(400) and, on a node hosting the type, a payload its decoder cannot read (422); both show in the sheet. The 1 MiB limit
is on the editor's text, not the server's body limit. Cancel during a send drops its answer, and the message may still
arrive; the pane shows it on its next read. The message is sent due now, with no correlation id, headers, retry or
retention policy.

## Blocking

None.

## Status

The app guide text above is proposed for the docs session. The 0.26.0 changelog and the inspector's crate rustdoc
name Send Message. The 12 send tests passed on virtual time, and the inspector's 191 tests. The workspace passed 4,076
tests with PostgreSQL; the docs test and the doctests passed 11 and 55. Formatting, workspace Clippy with warnings
denied, and strict OpenSpec validation passed. The native sheet's manual check list was not run.
