Verdict: not yet

Blocking
1. The local check does not match the server's (design 2). The server pins its options in `cicada-axum/src/schema_check.rs`: draft 2020-12 whatever `$schema` says, no `format`, `NoRetriever`. It also lets through a schema it cannot compile. "Same crate and version" with default options would disable Send for payloads the server accepts (a draft-07 `format`), and block Send forever on a schema that cannot compile (a bad `pattern`, an outside `$ref`). Decide: share the server's validator (a pub fn in cicada-axum, already reached through cicada-app, but a public API change outside the stated Impact), or copy the options and add a parity test against the door. An unusable schema shows a note and leaves Send enabled.
2. Where the check runs is not decided. The door only asks `is_valid`, because finding places has no bound (decision 8), and it runs on a 64 MiB stack (decision 11). Listing "the first few errors" per keystroke on a 1 MiB payload can freeze the UI, and a deep paste can overflow the stack and end the app. Decide: compile once per chosen type; check off the main thread on a large stack; one check in flight; stale results dropped; on a mismatch, only the first place.
3. Duplicate sends. The client keeps one id across its own retries, but after a final timeout a second Send makes a new id. Keep the sheet's id until the type or payload changes, and say what a 200 (duplicate) shows. Also say whether an alert keeps the sheet and payload.

Notes
- NSTextView: turn off smart quotes, dashes and text replacement (smart quotes break JSON). Escape there runs `complete:`.
- Cancel as default: Return in the pop-up closes the sheet and loses the payload.
- The 1 MiB text limit is not the body limit (envelope, configurable, compact re-encoding). Say a 413 goes to the alert.
- "Shows under Mailbox" fails when a turn takes the message first.
- The Types cache can be stale. A failed /types read, or a type missing from it, has no scenario.

Not checked: other harness reviews; AppKit behavior beyond memory.
