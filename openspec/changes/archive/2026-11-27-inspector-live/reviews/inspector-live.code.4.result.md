Verdict: fix needed

1. appkit.rs:1287–1293 and live.rs:447–452. Switching contexts on Overview sends Show with the new visit before Source. Show can reopen the old server’s stream under the new visit. If Source is delayed, that stream’s resync passes the visit check and refreshes the new server. A two-server scratch test reproduced the extra /stats request. Update the source and visit together.

Notes. model.rs:399–402 fences batches from the original visit. actors.rs:1452–1459 drops pending pane reads when the actor lookup returns 404. The biased select polls the lookup first. While it is pending, the other reads still poll and queue for slots. The gone-actor test at soft_refresh/live.rs:567 could time out if the 404 cancels the mailbox before it arrives. Hold the 404 until the mailbox arrives. outcomes.rs:463 skips finished tasks, but a superseded count can still finish without waking an active waiter if the newer count already published.

I ran only the targeted scratch test. I did not run the suite, gate, E2E or AppKit checks.
