Verdict: not yet

Round 1 findings: all three fixed. Stop Now is split into worker-stop-now (order is right). Quit, Ctrl-C and system terminate are defined (timer polls, NSApp.stop plus an empty event, NSTerminateLater). The run-time 14.0 check and 14.0-14.3 subtitle fallback are decided. Notes are taken.

Blocking
1. Quit waits for `Offline` only (design D3, D9, spec "Quit waits"). Against lib.rs, `finish` sets `Failed` when `run_with` errors, also while Stopping (and a Failed tray has no connection to stop). Quit, a system terminate (logout hangs on NSTerminateLater) and a pending context pick then wait forever. Wait for "no connection" (Offline or Failed), quit at once from Failed, and add a scenario and a state-machine test (Stopping ends in Failed; Quit from Failed; Quit clears a pending pick).

Notes
- The spec has no scenario for the run-time refusal before 14.0 or for the 14.0-14.3 subtitle fallback (it names the refusal only in the requirement text). The proposal still says "deployment target of 14.0", against D2.
- Say which thread owns the Runtime and how its background thread ends (shutdown signal, join, `shutdown_timeout`). Drop the entered-handle guard first. Ctrl-C and Quit while the menu is open: NSApp.stop only returns once menu tracking ends. A long grace period makes logout or a second Quit slow, and there is no force-quit until worker-stop-now; say so in the rustdoc.
- The status item's dimming must follow state changes at 250 ms, not the 1 s clock.

Questions: none.

Not checked: AppKit behaviour (from memory), objc2 availability attributes.
