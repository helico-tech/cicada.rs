# 0045. Docs handoff between the loops and the docs session

Date: 2026-11-14

## Status

Accepted. Adds documentation coordination to ADR 0030. Preserves the owner's documentation ownership rules and
land/release approval, and changes no library guarantee, API, wire format or schema.

## Context

The docs session owns handwritten user documentation. Builders cannot repair those files themselves when source
changes break doctests, links, guarantee references or skill routes. Waiting for a post-merge docs scan would let
main break. The existing ownership hook permits integration of exact content prepared on the owner's branch.

## Decision

Every change includes docs-handoff.md with its version, user-facing source changes, proving tests/examples, destination
files/headings and substance, caveats, blocking checks or an explicit none, and actual verification/docs status.
No-impact changes state why; release versions still reach docs for installation examples. The reviewed source is
identified by the review request's head, not a self-referential commit hash in the handoff. Behavior-changing fixes
update the handoff. After landing it travels with archived reviews for the docs session to reconcile against source.

The builder continues owning source, rustdoc, worked example code, consumer skill routes/version, ADRs, generated
references and changelog. Docs owns protected handwritten prose, including its Rust blocks and guarantee table.
README release pins keep their narrow existing exception; install-guide pins belong to docs. Existing tasks asking
for protected edits become handoff work, but their verification requirements remain. No ownership hook changes.

Nonblocking guidance can be written after landing: a truthful handoff does not claim prose already exists. Blocking
checks cannot. Finish independent code, identify failing blocks and proposed replacements, and report blocked on
docs with the concrete source head. Do not edit protected files temporarily, skip checks or grant a code-review pass
with a known failure. Lack of owner availability is a dependency, not permission to bypass ownership.

The docs owner prepares fixes against that feature head and current base through its own workflow. The builder may
integrate only agreed exact owner-prepared blobs/deletions into the feature branch without merging another branch.
The staged content must still equal the current recorded owner branch when committed. Inspect complete diffs and
preserve unrelated edits. The handoff records the separate prepared artifact/head/paths where needed. A commit
subject alone is not authorization or proof of compatibility.

Combine required documentation fixes with the breaking code before the normal finishing checks and code review.
The commits introduced to main must not contain an intermediate known docs break. Source/base changes invalidating
prepared fixes require owner reconciliation, not a blind conflict resolution. Recheck identity and tests on the
combined result, then use the normal owner's land/release gate. Builders/reviewers gain no merge, push or tag authority.

The producer's own handoff requests the docs-side preparation procedure; it does not edit docs-owned maintenance
instructions or assume a new feature-branch scanner exists. Until the docs owner adopts its side, coordinate explicit
requests and keep blocking work unlanded. No new daemon, request database, access isolation or approval mechanism.

## Consequences

Additive changes can land with green checks and a complete handoff; breaking examples may wait for docs coordination.
The builder reports that distinction. Existing ownership tests exercise the blob exception; process scenarios are
verified by brief review/walkthrough, not tests matching prose strings. This workflow/config implementation follows
normal patch release policy; the separate docs-only maintenance exemption stays unchanged.
