Verdict: not yet

Blocking findings

- Design decisions 2 and 5. Use the server’s initial `Resync` as the single recovery trigger on open, reconnect and resume. Keep the fallback armed while waiting. HTTP 200 can precede feed readiness. Enter Live on the first valid event and measure the 60 s backoff reset from then.
- Decisions 3–5. Both layers remember refreshes. Assign one owner for a bounded pending batch and its 500 ms deadline. Define the 30 s deadline from refresh completion and rearm after skipped attempts. On pause clear queued work and fence automatic list and pane results. Test overlapping hints, timer and resync while paging or refresh is blocked.
- Decisions 3–4. Use normal soft-refresh pane reconciliation for all triggers. The hint-specific rule leaves this unclear and risks starving pane reconciliation during sustained hints. Add the selected Nodes `node(id)` read to the soft-refresh prerequisite. It currently names only the cached list. Test fallback reconciliation for out-of-prefix children and linked Outcomes.
- Spec. Permit manual refresh while paused. Bound scheduling on an idle page. The 10 s read budget makes the unconditional one-second pane result untenable.

Notes. Keep the subscription through local filter changes. A private async source trait returning `ClientResult<ChangeStream>` supports channel tests with gated handshakes, EOF and errors. Fits five tasks covering stream lifecycle, scheduler, refresh integration, pill and finishing.

Strict OpenSpec validation passed. Runtime and AppKit checks unrun.
