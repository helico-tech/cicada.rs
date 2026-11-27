Verdict: ready for tasks

Round 2's blocker is fixed. Messages and Outcomes stop at the page holding the old last message id, else at 300 rows or no more. Running Turns and Watches stop by their known key order. Both notes are fixed: Watches replaces state.rows and re-filters (matches watches.rs), and panes filled from rows refill while reading panes keep old content until the read answers.

Notes (none blocking):
- If the old last message was consumed, the read runs to 300 rows. This is correct, and the tests could cover it.
- Messages' seen set is rebuilt from both lists, as the seam rule says.

Not checked: AppKit at run time; Outcomes code and server order.
