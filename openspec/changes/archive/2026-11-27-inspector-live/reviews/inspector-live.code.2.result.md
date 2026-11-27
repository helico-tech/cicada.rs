Verdict: fix needed

1. types.rs:268,295–305,387–389. Hold a user-started schema format, let live refresh change the contract, then pause and release the formatter. Its replacement formats the refreshed pane without a ticket and publishes while paused. This blocks landing. Restoring the previous pane makes sense, but the owned path exposes the new schema menu before formatting and rolls back after Pause returns. Keep the previous pane visible until fenced publication.
2. live.rs:227–242,437–446,518–526. Pause and resume before the driver consumes Pause. A due old batch can start before either command, then Pause discards its completion handle without cancelling the read. Keep the gate closed until the driver processes the current resume.
3. model.rs:485–493,600–605. Actions deliver page switches and fence a page that stays hidden. Switch Actors → Nodes → Actors before the driver processes either switch and an old Actors batch passes the section check. Fence by visit.
4. model.rs:496–514, actors.rs:640–648. If ⌘R’s Actors list answers before its selected pane or running sample, the timer starts early. Track the whole reload. The global wait also delays the new page’s resync behind an old reload (live.rs:388–398).

The pending prefix, close ordering, Offline label and 0.16.11 handoff check out. Pause can wait on synchronous page work. I found no lock cycle or lock held across an await in these paths.

I did not run tests, the gate, E2E, AppKit rendering or VoiceOver checks. The selected-actor test checks model selection and generation. Scroll retention remains unverified.
