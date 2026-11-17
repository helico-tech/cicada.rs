# Design

The owner requested this producer-owned comment correction and resumed the design loop. The existing autonomous
decision and Opus 5.5 review preferences apply. This follows archived sample-heartbeat-watch (v0.20.4).

1. **Exact scope.** Keep the first sentence of ActorSystem::nodes rustdoc and replace its stale second sentence with:

   > For observability, including node-down alerts. The node table grants no authority to claim, move or
   > fence actors; leases decide who runs what.

   Wrap as ordinary rustdoc lines. This matches the existing NodeSnapshotSource adapter, architecture and ADR 0054.
   It describes a use of the returned view, not a promise that calling nodes publishes alerts or activates a monitor.
   Do not add a cross-crate rustdoc link from core to cicada-actors or a dependency just to name the consumer.
2. **No behavior change.** Leave the method, signature, status thresholds, running-turn reads, node retention,
   monitor and claim/pin/fencing logic untouched. The wording does not change guarantees or require an ADR. No
   semantic changes to wire/contract references, guides, headings, examples or skill routes are needed. The spec delta records
   only the rustdoc requirement; existing node-view behavioral requirements already describe observability.
3. **Verification.** Inspect the source diff to confirm it changes only this comment. Compile core rustdoc with
   `cargo doc -p cicada --no-deps` and retain the normal committed gate; do not add a text-matching test or new runtime
   tests for this prose correction. Strict OpenSpec validation checks these design artifacts, not implementation.
4. **Ownership and delivery.** One implementation task plus finishing checks suffices. Write docs-handoff.md recording
   the producer correction and that no handwritten-guide fix is required. Use the normal next patch release (currently
   0.20.5), with changelog and standard version pins/generated version metadata. Do not extend the docs-session release
   exemption. This is release preparation, not permission to publish; the build loop obtains the owner's release yes.
   The builder changes the source comment; the build loop owns landing. This design session does not implement it.
5. **Archive note.** The node-view spec Purpose still says display only, although its normative requirement already
   allows sampled alerts. A requirement delta cannot replace the Purpose. Record in docs-handoff.md that the build loop
   should align that sentence to observability including node-down alerts when archiving; preserve the lease boundary
   and all behavioral requirements. This is spec editorial maintenance, not a guide change or a new feature.
