Verdict: fix needed

Blocking findings:

1. `examples/src/flow_resources.rs:162` acknowledges `Stopped` after an idempotent removal, but `Containers` (lines 68–73) does not require fencing or settling already-issued creates. A Start can submit an external create and return a timeout; Stop then removes the currently absent container, acknowledges, and cleanup releases capacity and records cancelled. The original external create can subsequently finish, leaving an unaccounted live container. The actor's `stopped` flag only blocks a later durable Start; it cannot fence that outstanding external operation. This violates the example's resource-safety requirement. Reproduced against the reviewed artifacts with `/tmp/flow-finalizer-review/late_create.rs`: cancelled/capacity released/container absent, then original create completion leaves `example.sandboxes/j1` alive. Require removal to settle or durably tombstone accepted/in-flight creates before acknowledging, model that guarantee in the service fake, and add this delayed-completion test. The existing late-Start test covers a different ordering.

Notes:

- No additional blocker found in phase dispatch, atomic settlement, recovery, reply routing, or lookup stability. Reviewed the documentation handoff under decision 24a; the changed flow contract remains equal after stripping descriptions.
- 0.18.5 follows the agreed staged rollout and non-BREAKING proposal rule. Its observable change remains significant: Cancel completion precedes terminal status even with the default hook; the handoff documents this and the requirement for identical upgraded hosts.

Not checked: broad test suites were not rerun; relied on the reported workspace/PostgreSQL, clippy, formatting and final actor checks. Per-feature checks and Docker E2E remain release checks. Ran only the outside-repository reproduction and read-only comparisons.
