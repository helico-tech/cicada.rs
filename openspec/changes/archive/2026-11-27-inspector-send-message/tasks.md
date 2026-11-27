# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-actions` (its one-action-at-a-time rule, target
checks, alert text and fresh read; rebase onto it when it landed first). Follow `design.md`; the spec is
`specs/inspector/spec.md`; the design reviews are `.reviews/inspector-send-message.design.*`.

- [x] 1. **The types for the pop-up.** The message types of the actor's type from the Types cache or one `/types` read;
  a failed read with Retry; "type not registered"; "this type takes no messages" with no Send (decisions 1 and 5).
  Verify: the in-process server tests of each case.
- [x] 2. **The local check.** The schema compiled once per chosen type with the server's settings (draft 2020-12
  forced, no `format`, a retriever that loads nothing; a comment naming `cicada-axum`'s `schema_check.rs`); a schema
  that cannot be compiled shows its note and leaves Send on; on a 64 MiB-stack thread, one check at a time, 300 ms
  after the last edit, stale results dropped; Send disabled at once on an edit; size (1 MiB), parse and depth (64
  levels, `cicada::runtime::limits::MAX_DEPTH`, counted as the server does) before `is_valid`; "does not match the
  schema" on false (decisions 2 and 3). Verify: the parity test against an in-process server on a type no actor hosts,
  the 65-level and 1 MiB payloads, a stale result dropped.
- [x] 3. **The message id and the send.** One id per payload, fresh when the type or the parsed payload changes;
  `send_message` with that id; 202 "sent <id>", 200 "already sent <id>", both closing the sheet and asking for a fresh
  read; 404 "actor not found" in the sheet; 422 or 400 in the sheet with the server's text, cleaned and cut as
  `inspector-actions` does, the payload kept; 413 and other refusals as an alert with the sheet kept; a context switch
  or close dropping the result; one action at a time with `inspector-actions` (decision 4). Verify: the same id re-sent
  after a timeout shown as "already sent", an edited payload after a lost answer sent with a new id, and each answer's
  mapping.
- [x] 4. **The AppKit sheet.** Send Message… in the actor pane; the pop-up, the read-only schema and the plain-text
  editor with smart quotes, dashes and text replacement off; no default button, ⌘Return sends, Escape cancels also from
  the editor; every `unsafe` with its reason, borrows released before AppKit calls (decisions 1 and 6). Verify: `cargo
  build -p cicada-inspector`, `cargo clippy -p cicada-inspector --all-targets`, and the manual check list below, each
  item noted as done, run by code (not seen), or not done in the report.
- [x] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names Send Message; `docs-handoff.md`
  carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or the handwritten docs. Then `cargo fmt --all
  --check` (lines ≤120, comments included), `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `cargo
  test -p cicada-examples --test docs` and `--doc`, `openspec validate inspector-send-message --strict`, commit. The
  change loop runs the gate at landing.

Manual check list (task 4): with a node running, send a valid message and see the pane read again; a payload that
fails the schema keeps Send off; ⌘Return sends, Escape cancels also from the editor, Return never closes the sheet;
typed quotes stay straight; a server refusal shows in the sheet with the payload kept; light and dark; VoiceOver reads
the pop-up, editor and buttons.
