Verdict: ready for tasks

Round 1's blocking finding is fixed. Decision 4 now fixes the order: after the usage errors, the on_start/task refusal, http_target and the built-in store check, just before the store or server wait. resolve and resolve_http are split into parse and apply, which also answers note 1. The spec requirement now says "after every check ... before the wait". It has scenarios for a bad flag and for an operator command. Notes 2 to 5 are handled: decision 5 covers one run per hook and --node-id other, and the Docs section names the module rustdoc and Debug. Decision 1 says the hook is sync because it does file I/O.

Blocking: none.

Notes for the tasks:
1. check_built_in runs today inside node.rs open_unless_stopped, after the point where the hook must run. A task must move it (or a copy) into lib.rs, before the hook, using the parsed StoreSpec. Keep the call in open_unless_stopped or drop it, but do not leave two diverging paths (node.rs lines 526 and 540 use it for the error text).
2. The HTTP worker has no store, so "built-in store check" does not apply to it. Say so in the task, so the order is clear for both modes.
3. Add a spec scenario for "bad --context: exit 1, hook not called" and "store feature not built". They are only in the Tests list now.
4. Handing in the dead slot: the example's identity_slot.set(..).ok() is fine given decision 5.

Questions for the owner: none.

Not checked: Factory's code; nothing built or run.
