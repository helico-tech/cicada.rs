Verdict: pass

No blocking findings.

Checked against the code:
- Turn sends (turn.rs:883, completion.rs:253) set the handled message id. Operator retry (system.rs:1543) sets the original id, as ADR 0015 says.
- Stopped notices: `Removal::of_commit` and `Notice::absent` set the turn's message id. That covers stop, delete, removal with an ancestor (cause `parent`) and `watch_existing`. `Removal::delete` sets None for a delete outside a turn.
- Failed notices set None (`Notice::failed`).
- The new `OutgoingMessage::caused_by` doc matches too.
- Only rustdoc and tasks.md changed. docs/openapi.json changed only in the two `causedBy` descriptions, which match the rustdoc. Added lines are 120 columns or less. No protected docs were touched.

Notes:
- `api/leases.rs:489` still says "or null", which is vague but not wrong.

Not checked: cargo doc, fmt, tests and the OpenAPI drift test were not run.
