Verdict: ready for tasks

Blockers: none. Both round 2 blockers are fixed.

1. No unstoppable check: decision 3 asks only `is_valid` and shows a summary, as the door in `crates/cicada-axum/src/schema_check.rs` does (jsonschema 0.61.2). The spec scenario changed to match.
2. Depth: checked before the schema. `cicada::runtime::limits` is a public module and `MAX_DEPTH` is public, so no API change is needed. The inspector copies `walk` in a few lines: a scalar is 0 deep, each array or object adds 1, 65 is refused. serde_json's default limit of 128 levels keeps that walk safe. The order matches `send_message_checked`: body size, parse, envelope, depth and NUL, duplicate id, schema, then the hosted decoder.

Notes:
- Decision 3 says the server's 422 text "names the pointer". It does not: the door's text is "does not match the registered schema; GET /types/<name> has it". Drop that bit, and do not test for a pointer.
- The server's NUL check (400) and a hosted actor's decoder (422) are not checked locally. They reach the sheet as server errors, which is fine.
- Round 2's notes are all covered: a fresh id on a type or parsed-payload change, Send disabled at once on an edit, and the parity test on a type no actor hosts.

Not checked: AppKit behaviour; no tests run.
