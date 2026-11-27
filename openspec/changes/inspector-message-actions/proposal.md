# Proposal

## Why

Operators want to retry, cancel or deliver a message now from the inspector, one at a time or several selected rows at
once (owner's proposal for `inspector-actions`, split off by its design, 2026-11-27).

## What Changes

- The message panes (the Messages pane, and the Outcomes pane with its followed links) get Retry…, Cancel… and Deliver
  Now, by the state of the message shown (`retry`, `cancel`, `deliver_now`).
- Several rows at once: Retry… on the selected Outcomes rows (`bulk_retry`) and Cancel… on the selected Messages rows
  (`bulk_cancel`), at most 1,000 ids. Outcomes are finished, so cancel belongs to the pending messages of the Messages
  page, not to Outcomes (changed by the design).
- A retry sends a new message and may repeat external work, so it asks first and names the message's retries still
  kept; a bulk retry skips rows with a retry still kept.
- Refusals show as alerts with the server's message, as the actor actions do.

## Order

After `inspector-send-message`. No following change.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the message actions, multiple selection in two tables, bulk). Tests against an in-process
server; a manual check list. Docs handoff: the app guide.
