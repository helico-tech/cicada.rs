Verdict: fix needed

1. Follow a cause from outcome A, select outcome B, click Show Actor, then go Back. The pane reopens A’s cause because navigation saves the old link even after B replaces it. Save B’s snapshot on departure and preserve the link when a status reset clears its pane. [model.rs:431](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-fff7d422-5d05153ed149c1d2/crates/cicada-inspector/src/model.rs:431)
2. A `Stopped` notice for an absent `watch_existing` target records the checking turn. Include that case in the cause wording. [docs-handoff.md:53](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-fff7d422-5d05153ed149c1d2/openspec/changes/inspector-outcomes/docs-handoff.md:53)

Verified the original status-switch path, generation-guarded badge publication, initial Caused loading, deferred row selection, retry attribution, empty-cause label, and AppKit borrow by inspection. Hold the first Caused response in the loading test so the watch can’t skip that state. Tests and manual UI checks were not run.
