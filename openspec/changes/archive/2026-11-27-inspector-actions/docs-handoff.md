# Docs handoff for inspector-actions

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The Actors pane in the macOS inspector now uses the existing Cicada HTTP API for mailbox pause and resume, running
turn cancellation, mailbox purge, and deletion with descendants. The portable tests in
`crates/cicada-inspector/src/tests/actions.rs` exercise the real client calls, confirmation text, and notices and
alerts. The tests in `crates/cicada-inspector/src/tests/actions/reconcile.rs` order the replies and verify that the
inspector starts fresh reads after successful writes and drops older outline results after Delete.

## 1. Actor actions

**Where.** `docs/guide/app.md`, `Inspector on macOS`, in the description of the Actors pane.

**Text.** Remove “read only” from the description of the Actors pane. The pane offers **Pause** or **Resume** for the
mailbox, **Cancel Turn…** while it shows a running turn, **Purge Mailbox…**, and **Delete…**. These use the same
routes and context credentials as `cicada`. Pause lets a running turn finish. Cancel Turn, Purge Mailbox and Delete
open a sheet naming the target. Return and Escape both cancel the sheet. Purge leaves a running message in place.
Delete removes the actor, every descendant and all their mailboxes. Pause, Resume, Purge Mailbox and Delete
show “actor not found” when the actor is absent. Cancel Turn shows “no turn was running” when the server finds
no running turn. A refusal opens an alert even if the selection has changed.

**Caveats.** Actions use the address named in the sheet, which can belong to a recreated actor when the server
gets the request. Cancel Turn names the last-read message. The server requests cancellation of whichever turn is
running when it receives the request. “Cancel requested” acknowledges the request and the handler may already have
finished. Purge counts pending rows among the mailbox's first 50 rows as last read. Delete counts loaded direct
children. The sheets indicate when a count is a lower bound or has not been read.

## 2. Reading after an action

**Where.** `docs/guide/app.md`, `Inspector on macOS`, with the Actors pane and Live controls.

**Text.** A successful write schedules a new read even when Live is paused. A timeout still shows an alert and
schedules that read because the server may have applied the write. If paging is in progress or failed, the selected
pane can be read on its own. An acknowledged Delete drops the actor and known descendants from the loaded outline
immediately. The pane shows “not found” if it still shows the deleted actor. Selecting another actor before the
delete replies keeps the newer pane.

## Blocking

None.

## Status

The app guide text is proposed for the docs session. The 0.26.0 changelog and inspector rustdoc carry the update.
A refusal with a canonical `about:blank` problem and no instance shows only the HTTP status. The client also uses
that shape for arbitrary error bodies.

The focused actor action tests passed 16 of 16 on virtual time. The workspace passed 4,061 tests with PostgreSQL.
The separate docs test and doctest runs passed 11 and 55 tests. The macOS inspector built and Clippy, formatting,
and strict OpenSpec validation passed. The model checks ran by code. Native UI behavior still needs a manual check.

After code review round 1, the inspector passed 178 tests and one doctest. The new tests cover refusals that arrive
after reselection or Refresh and a timed-out write that the server had already applied. Workspace Clippy with
warnings denied, formatting, and strict OpenSpec validation passed.
