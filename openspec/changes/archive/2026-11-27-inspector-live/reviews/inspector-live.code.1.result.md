Verdict: fix needed

1. live.rs:369,380 and appkit.rs:1286. Change the Actors prefix, then pause or switch context during debounce. The driver erases the pending prefix and AppKit has recorded it as sent. Resume or the new context follows the old prefix. Preserve the latest requested prefix.
2. appkit.rs:647,1279. Switch Actors → Nodes → Actors between ticks and the driver never sees Nodes. A due Actors batch can also start after leaving Actors. Deliver switches from the actions and fence stale work.
3. appkit.rs:1059,2118 and live.rs:454. A reply can publish after the Pause click queues its command. On close, a due batch can spawn a read after the session cancels its reads. Fence publication at pause and block new reads before shutdown cancellation.
4. types.rs:151,287,359. Pause while formatting a schema changed by a live refresh. The formatting can still publish into the pane because it checks only the pane generation. Fence refresh-owned formatting.
5. appkit.rs:1239. A ten-second ⌘R reload puts the fallback twenty seconds after completion. Arm thirty seconds from completion.
6. live.rs:113. Open with an invalid context and the disabled pill reads “Live” before any event. Give Off its own label.
7. docs-handoff.md:46. A 0.16.11 server already serves /changes (CHANGELOG.md:2055). Correct the minimum server version.

Notes. Connecting and the cached, generation-checked viewport fit the design. The handoff otherwise fits ADR 0045. Tests omit actual scroll preservation, a manual reload while paused, and stale marks after Retry.

Not checked. Test execution, gate, E2E, AppKit rendering or VoiceOver.
