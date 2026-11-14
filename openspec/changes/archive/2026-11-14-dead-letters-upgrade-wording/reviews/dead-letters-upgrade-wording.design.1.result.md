Verdict: ready for tasks

Blocking findings: none.

The correction matches the implementation. `convert` calls `check`, and `reset` repeats it before deletion. `check` rejects repeated export addresses, NUL characters in state strings/keys, excessive state depth, and failed `DeadLettersState` decoding. `restore` calls `create_actor`; its schema callback accepts everything, and typed decoding occurs only on a hosting node. The example's independent typed preflight therefore matters on its handler-free management node.

The proposed qualification accurately limits the schema/decoder test to payload fields and avoids claiming whole-state equivalence. All six existing upgrade scenarios remain, with the legacy-deep scenario now locating validation before deletion. Append-only archive corrections preserve the original record and completed task count. The change fits its stated scope, respects documentation ownership, and is compatible with its immediate predecessor's handoff rules; no following proposal exists.

Notes: none.

Questions for the owner: none.

Checked the change artifacts, current dead-letters spec, archived claims, relevant architecture and ADRs 0026/0029/0030/0043, implementation, existing tests, upgrade guide, and adjacent proposal. `openspec list`, `openspec show dead-letters-upgrade-wording`, and `openspec validate dead-letters-upgrade-wording --strict` completed successfully; strict validation reports the change valid.

Not checked: execution of Rust tests, rustdoc, workspace/release checks, or the future implementation diff. No runtime or production migration was performed.
