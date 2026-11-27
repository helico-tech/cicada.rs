Verdict: not yet

1. **Decisions 1, 4 — Target and serialization.** Capture context, client, address and pane generation. Revalidate rendered controls and sheet confirmation before dispatch. Gate all buttons from sheet opening through request completion, across reselection. Fence alerts and refreshes on context switch or close. Address-only writes can hit a recreated actor.
2. **Decision 4 — Cancel.** The POST targets whichever lease the server finds, potentially a successor. Make that explicit or defer Cancel Turn. Show “Cancellation requested” for 202, even after handler return. False means no valid lease, including a missing actor.
3. **Decision 3 and spec — Results.** Pause/resume/delete return 204 or typed 404 as false. Purge returns 200/count or typed 404 as None. Use local absence text. Render remaining API errors, including auth/store failures. These routes have no native 409/422. Update the existing read-only Actors requirement.
4. **Decision 3 — Reconciliation.** Coalesce a post-write refresh behind pre-write reads even with Live paused. Define behavior when paging blocks refresh. Evict deleted occurrences beyond refresh bounds and reconcile “not found” with selection clearing.
5. **Decision 2 — Sheets.** Label actor-mailbox and direct-child counts separately. Use last-seen counts, loaded-page counts when partial, and “unknown” when unread. Purge spares the running message. Delete includes descendant mailboxes and running messages. Bind Return and Escape to Cancel explicitly.

Five tasks fit with existing routes: results, coordination, confirmations, AppKit, verification and handoff. Test gated races and missing actors in process. Runtime and keyboard checks unrun.
