Verdict: ready for tasks

Blocking finding from round 2: fixed. D3, D9 and the spec requirement now wait for no connection (Offline or Failed). The spec has a "A stop that fails" scenario. The state-machine tests cover quit from Failed, a stop that ends Failed, and a pick from Failed. This matches lib.rs: `finish` clears the connection for both states, and `connect` accepts when none is held. The thread and shutdown order, tracking cancel, the rustdoc line on the long grace period, the 250 ms dimming and the proposal text are all handled.

Notes (none block):
1. The spec requirement names the 14.0-14.3 title fallback but has no scenario. Add one; the refusal scenario is there.
2. The test list lacks "Quit clears a pending pick", which round 2 asked for. Add it to the state-machine tests, and add a spec scenario for a pick from Failed.
3. "Cancels the menu's tracking" (D3) has no manual check in the list. Add "Ctrl-C while the menu is open".

Questions: none.
Not checked: AppKit behaviour, objc2 availability.
