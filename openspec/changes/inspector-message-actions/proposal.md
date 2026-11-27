# Proposal

## Why

Operators want to retry, cancel or deliver a message now from the inspector, one at a time or the selected outcomes at
once (owner's proposal for `inspector-actions`, split off by its design, 2026-11-27).

## What Changes

- Message pane (Messages, Outcomes' followed links): Retry, Cancel and Deliver Now, by the state of the message
  (`retry`, `cancel`, `deliver_now`).
- Outcomes: retry or cancel the selected rows at once (`bulk_retry`, `bulk_cancel`, at most 1,000 ids), with a sheet
  that names what goes for bulk Cancel.
- Refusals show as alerts with the server's message.

To decide in the design: multiple selection in the Outcomes table, and how a bulk result is reported.

## Order

After `inspector-send-message`. No following change.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the message actions, multiple selection, bulk). Tests against an in-process server; a manual
check list. Docs handoff: the app guide.
