Verdict: ready to land

No blocking findings. model.rs:399,554 and live.rs:241,471 pair the client with its visit. On a context switch, the driver replaces both before opening the stream. An old resync cannot refresh the new context because model.rs:498 checks the visit. Within one context, page and subscription-prefix changes still send Show. tests/live.rs:131 checks the switch against two real streams.

Notes. outcomes.rs:440 clears the finishing task's slot by ID and wakes the waiter even when a newer count already landed. tests/soft_refresh/live.rs:568 waits for the mailbox request before releasing the 404. At line 563, waiting for details alone leaves a test race: if Refresh cancels the initial mailbox before it reaches the server, the reload consumes the first mailbox response and the test waits for the second. Wait for the initial pane to finish before reloading. live.rs:66,342 still refer to the now test-only Live::show.

I did not run tests, the gate, E2E, or manual AppKit checks.
