Verdict: ready for tasks

Blocking findings: none.

The written design preserves the core notice and ordinary depth checks. Current code confirms the three-container dead-letter overhead, the notification-versus-state distinction, and redrive's separate validation/store/commit failure paths. The explicit payloadJson field avoids reinterpreting user payloads; presence tracking, schema parity and deferred parsing address null and damaged-letter cases. The upgrade accounts for registration conflicts, existing hosts, descendants, incoming notifications, outgoing watches and lost instance identity. No conflict found with the landed HTTP-worker predecessor or the listener-probe successor.

Notes:
- In design.md, “Proofs and scope”, make the depth-walk boundary cases include empty arrays/objects and mixed nesting. Containers count even when empty; the existing core limits tests show the convention. This is a test detail, not an unresolved design choice.
- The Sleep comment and event lifecycle-test repairs are independent of the payload behavior. Keep them confined to the stated repairs when writing the five implementation tasks.

Questions for the owner: none.

Checked the change artifacts, relevant architecture/ADRs and existing specs, adjacent proposals, notification construction/decoding, dead-letter implementation and tests, state validation, HTTP-worker client, dependency declarations, watch APIs and SQL type-removal guards. `openspec list`, `openspec show deep-failure-notices --json` and `openspec validate deep-failure-notices --strict` ran; strict validation passed.

Not checked: implementation (not yet written), runtime stack experiments, actual numeric round trips, database upgrade rehearsal or Rust tests. No workspace suite run.
