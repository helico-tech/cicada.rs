Verdict: fix needed

Paths are relative to crates/cicada-inspector/src.

1. actors/actions/send.rs:176 and actors/actions/send/check.rs:117. Cancel during a slow check and reopen. The old worker keeps running while the new one starts. Share one worker across sheets and discard abandoned jobs.
2. actors/actions/send.rs:280. Edit draft A to B while A is sending. A's 422 appears against B, success discards B, and a timeout promises the same id even though Send uses B's new id. Freeze the draft while sending.
3. actors/actions/send.rs:276. Queue Send for actor A and select B before the spawned task runs. The task still posts to A. Recheck selection at dispatch as the other actions do.
4. actors/actions/send.rs:451. After A times out, choose a type whose schema rejects the payload, then return to the original type. Send reuses A's id despite the type changes. Rotate the id on selection.
5. actors/actions/send.rs:423. Paste and switch types within 300 ms. The type change checks the edited payload immediately. Preserve the edit deadline across the change.
6. actors/actions/send.rs:292. Cancel during a pending POST, then press Purge. Cancel releases the slot immediately while the send may still be processing. Suppress the answer and keep the slot until the request settles or times out.
7. appkit/send_sheet.rs:243. A type without messages still displays disabled Send. Hide it.

Notes. The cache-miss read, spelling correction setting, clearing refusals on edits, timeout refresh, and module placement are acceptable. Cancel dropping the answer is acceptable with the slot held as above. All four spec scenarios have model tests.

Not checked. Cargo tests, gate, E2E, and the native AppKit checklist were not run.
