# Docs handoff for inspector-outcomes

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The macOS inspector has a read-only Outcomes page. It lists the finished messages by status and address prefix, shows
the store's failed count in the sidebar, and shows the selected outcome's error, attempts, retry policy, envelope and
payload in the pane, with its cause chain one step at a time. Each link of the chain is a place in back and forward.
No public API changed; the page lives in `crates/cicada-inspector/src/outcomes.rs` (model) and
`src/appkit/outcomes.rs` (window).

The in-process server tests in `crates/cicada-inspector/src/tests/outcomes.rs` cover the list by status and prefix,
paging by the server's cursor with duplicates kept once, the status switch's reset, the sidebar count ("…", the kept
value with the error, "—", the reset on a context switch), the pane from the row with the payload formatted on
selection, Caused by followed to pending, finished and expired messages, Caused paging with one request at a time,
Retry and empty pages followed, a failed linked read with Retry also after back and forward, Back to an outcome that
is no longer listed and its row selected once a page brings it, back and forward across a status switch, a context
switch clearing the history, stale reads dropped, and the failed count published as soon as `/stats` answers and not
by a refresh that a later one replaced.

## 1. The Outcomes page

**Where.** `docs/guide/app.md`, `Inspector on macOS`. Add after Messages, and update the list of implemented Browse
items (Running Turns and Nodes still say "Coming soon").

**Text.** Outcomes lists the finished messages of the actors under the address prefix in the search field, 100 at a
time as you scroll. A status switch in the toolbar picks Failed (the default), Cancelled, Done or All; All adds a
status column and includes purged outcomes. The switch is in the toolbar only on this page. Each row shows the message
type, the actor, the attempt, the finished time and the first line of the error; the tooltip has the whole error. A
message that appears on two pages shows once, as first read. Switching the status reads the first page again and
clears the selection and the pane.

The sidebar's Outcomes item shows the store's failed count, the number of failed outcomes that retention still keeps.
It is read with the Overview's refresh, where it shows as soon as `/stats` answers, and with ⌘R on Outcomes; it is not
live. It shows "…" until a read answers and again after a context switch. When a read fails, the last count stays
and its tooltip names the error. "—" means the server is older and sends no count, or the first read failed.

Select an outcome to see, read only: the status and error, the attempt and failures, the last retried error, the
retry policy, the envelope (id, target, sender, correlation id, finished time and headers) and the payload as
highlighted JSON (cut at 256 KiB). **Show Actor** goes to the actor on the Actors page.

**Caveats.** ⌘R on Outcomes reloads the list from the top, reads the failed count again and clears an outcome from the
pane; a linked message in the pane is read again. A failed page shows **Retry** in the footer. Retry and cancel of a
message are not in the inspector yet.

## 2. The cause chain

**Where.** `docs/guide/app.md`, `Inspector on macOS`, under the Outcomes page.

**Text.** The pane follows the chain one step at a time, by the cause each message records (`causedBy`). **Caused
by** links to the message recorded as this one's cause: the message whose turn sent it, the original message of an
operator retry, or, for a `Stopped` notice the store wrote, the message whose turn stopped or deleted the actor. It
says "No recorded cause" when there is none: a send from outside an actor, a `Failed` notice, or a stop from outside
a turn. **Caused** lists the messages that record this one as their cause, one level down: pending or running ones,
and finished ones whose outcome retention still keeps. It shows the first 50, with **more…** for the next ones and
**Retry** when a page fails. It says "None" when there are none, and "not found" when the message is unknown or past
retention.

Following a link is a new place in back and forward. The pane reads that message fresh and shows it whether it is
pending, running or finished. A message past retention shows "not found"; a read that fails shows its error with
**Retry**. Back to an outcome shows it as it was read, even when its row is no longer listed (another status, a later
page or another prefix); its row is selected when it is listed, or once scrolling brings it, unless you selected
something else first. Forward to a link reads the message again, also after you switched the status.

**Caveats.** A context switch clears back and forward. The chain is not a snapshot: messages caused later can appear
when you return.

## Blocking

None.

## Status

Proposed app guide text; no protected prose was edited. The checks are listed in the change's report and in the
tasks: crate build and Clippy, workspace Clippy, formatting, the workspace tests, the docs tests and doctests, and
strict OpenSpec validation. A temporary AppKit driver, not committed, exercised the window by code against an
in-process server (run by code, not seen): the Failed list and the switch to Cancelled, Done and All with the status
column, the status switch leaving the toolbar off Outcomes, the sidebar count and its VoiceOver label, the pane's error,
envelope and payload, Caused by and a Caused link with back and forward, Show Actor and back, the Refresh action, Down
arrow selection, and a stopped server's failed page and count. Light and dark appearance, the ⌘R key equivalent and
VoiceOver still need a manual pass.
