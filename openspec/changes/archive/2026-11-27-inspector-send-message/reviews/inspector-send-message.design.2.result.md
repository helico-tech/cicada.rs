Verdict: not yet

1. In design.md decision 3, jsonschema 0.61.2’s synchronous validate has no cancellation and can collect entire branch errors. A 2 s timer only stops waiting. The sole worker can stay occupied and keep allocating. Use is_valid alone and show the mismatch summary. Revise the pointer scenario and tests. A hard deadline requires cancellable execution.
2. Decision 3 needs the server’s 64-level payload limit before schema validation. The server sizes its stack for that limit (ADR 0029).

The validator settings, compile-failure fallback and parity test match the server. The revision covers substitutions, Escape, no default button, a 413 alert preserving the draft, fresh-read wording, and both /types cases.

For the tasks, mint a fresh id when the message type or parsed payload changes. Reuse it on unchanged retries. The server ignores payload on an id match for the same target and type and returns 200 within retention. Map that to “already sent.” Test sending an edited payload after a lost acknowledgement, invalidate Send immediately on edits, and use an unhosted type for schema parity.

AppKit behavior and runtime tests were not checked.
