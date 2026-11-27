Verdict: pass

Reviewer: claude-reviewer (other family than the builder, gpt-6.2-sol).

## Blocking

None.

## Notes

1. `crates/cicada-inspector/src/appkit.rs` `window_will_close`: an embedded native close (⌘W or the close button)
   drops the slot's `Retained<Target>`, so the Target and its `NSWindow` are released at the end of
   `windowWillClose:`, while AppKit is still inside `-close`. This is the usual "release the controller in
   windowWillClose" pattern and AppKit keeps the window alive through `-close`. Still, add "close by ⌘W and by the
   close button, then reopen, no crash" to the manual list.
2. `menu.rs` `system_termination_defers_until_the_connection_ends` lost its Offline/Failed `Now` lines. They moved to
   `every_quit_request_closes_the_inspector_before_letting_the_host_act`, because an immediate terminate now sets the
   quit flag (`Quit::Finishing`), as the design review asked. That is intended, not a test bent to pass.
3. `scripts/ci/dependency-trees.sh`: the two added `grep` lines are 122 characters (the file already had longer
   ones).
4. The new inspector tests are `#[tokio::test]` with event waits bounded by 10 s timeouts, and no sleeps. This
   matches the existing switch test. The lifecycle test drives `WindowSlot` with `Rc<Session>`, not the AppKit
   `Target`, so `show`/`close`/drop on real windows is covered only by the manual list.
5. CHANGELOG says Quit "cancel[s] its refresh", while the handoff says it "requests cancellation". The handoff's
   wording is more exact.

## Checked

Teardown shared and run once (timer invalidated, `Session::stop` bumps the generation and aborts on the host's
handle, delegates cleared before `close`, no `windowWillClose` on a host close). Standalone still stops the app on a
native close. `Embedded` never sets the policy, delegate, main menu or run loop; it only adds its timer and calls
`activate`. Keys go through `performKeyEquivalent` with a key-window check. Feature wiring, the item disabled and
`show` blocked while quitting, the context passed (any connection, else None). All four quit paths set the flag, then
close the inspector. Borrows are released before AppKit calls. SAFETY comments present. Gate scripts, CHANGELOG,
handoff and versions (0.26.0) checked; no protected docs edited. I ran the tray tests with the feature (55 pass) and
without (54), the inspector tests (17 + 14 + 1), and clippy for the tray with the feature: all clean.

## Not checked

Any GUI behaviour (the task 3 manual list is still pending), the full gate, PostgreSQL tests, non-macOS builds.
