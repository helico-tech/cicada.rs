Verdict: fix needed

## Blocking

1. `crates/cicada-inspector/src/actors/actions.rs:262-268`: a refusal is lost when the selection changes. `feedback`
   is `selected.then(...)`, so an alert (or a timeout) is dropped whenever the target is no longer selected. Decision 2
   drops results only on a context switch or a closed window. Decision 5 and the spec ("another refusal an alert with the
   server's message") want the alert. Decision 6 fences only the Delete notice by selection. Scenario: confirm Purge on
   A, the server holds the request and then answers 403. Meanwhile the user selects B or presses Refresh. No alert
   appears, and the user believes the purge ran. Fixed: an alert or timeout shows whatever the selection is (not after
   a context switch or close). Only the success notices stay bound to the pane's generation. Add a test for a refusal
   that answers after a reselection.

## Notes

- `reconcile.rs:60-64`: a Delete that answers while a descendant is selected clears `selected` but keeps that pane.
  It then shows a deleted actor with no buttons and no reread. This fits the design's wording, but it is odd.
- `actions.rs:259`: the 10 s local timeout shows "timed out" and starts no read. The server may still have done the
  write.
- `alerts.rs:42,54`: every header value and the URL user name count as secrets. Short values such as `1` or `admin`
  are cut from all alert text. This is safe but garbles the text.
- The builder's edge case (`alerts.rs:92-97`) is acceptable. Cicada's own problems use `urn:cicada:` types. Only a
  host middleware's canonical `about:blank` loses its detail, and the client cannot tell it apart from a raw body. The
  safe side is right. Keep the caveat in the handoff.
- Handoff: "An absent actor shows 'actor not found'" leaves out Cancel Turn, which shows "no turn was running".

## Not checked

The AppKit behaviour (Return, Escape, the destructive button, VoiceOver, light and dark) was not run. I did not
verify that `NSEvent.removeMonitor` is safe when it runs inside the monitor's own handler (the sheet ends in it, and
its completion drops `Dialog`). Gate, workspace tests and clippy were not run. I ran only the 16 action tests, and
they pass.
