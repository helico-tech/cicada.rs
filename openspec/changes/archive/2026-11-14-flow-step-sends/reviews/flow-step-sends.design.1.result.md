Verdict: ready for tasks

Blocking findings: none.

The outbox fits the existing public Turn operations: raw creates validate before buffering, raw sends preserve normal metadata and observer headers, and handler or commit failure discards all effects and progress. The proposed error mapping preserves node/refused/commit distinctions. The replay and first-latch rules extend the current runner without weakening G2, G4 or G6.

The reserved wire shape accommodates the adjacent reply, timeout, join, cancellation and finalizer proposals. Its interrupted-main/cleanup overlap is explicit, bounded and testable. The deployment plan correctly separates old-state decoding from registration compatibility and equal hashes from behavioral compatibility.

Notes:
- In design.md, “Upgrade and verification,” also name the Rust source breaks in migration notes: exhaustive matches on FlowMsg, FlowStatus and JournalEntry, and FlowState struct literals need updating. The breaking release already permits these changes.
- Include an event-start regression among the targeted checks: EventActor constructs FlowState::new as raw JSON, so the new defaults must preserve its ability to create and start a flow while its own contract remains unchanged.

Owner questions: none.

Checked: architecture, relevant ADRs, all change artifacts, adjacent proposals, existing flow/event/Turn/resolver/error code and guarantee references. OpenSpec list/show and strict validation passed. Not checked: implementation compilation, generated schemas or runtime tests; this is a design review and no workspace suite was run.
