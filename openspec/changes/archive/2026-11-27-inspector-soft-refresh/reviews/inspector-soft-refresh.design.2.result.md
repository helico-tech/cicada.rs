Verdict: not yet

Round 1's blockers are fixed in the text (range end, seam, generation, pane, snapshot, scroll, failed-read scenario, Nodes clash, size).

Blocking
1. Decision 1, Messages and Outcomes. "The id ... as their order is the server's" does not give a range end. The mailbox is ordered by (deliver_at, seq), not by id (MessageView, docs/architecture.md "Mailbox"). "Rows reach or pass the old last id" cannot be tested: if the old last message was delivered, the read never meets it and always runs to 300 rows. Use the old last row's (deliver_at, seq) as the key, or say "stop when a page holds the id, else at 300". Do the same for Outcomes. The seam rule (drop old tail rows already in the range) then follows by id.

Notes
- Watches keeps all rows in state.rows and filters them by prefix into list.rows (watches.rs filter). The soft refresh must replace state.rows and re-filter. Say so.
- Messages' pane is filled from its row and has no read. "Re-read without Loading" only matters for Types and Outcomes' link.

Not checked: AppKit at run time; Outcomes code.
