# Design review resolution

Opus 5.5 returned not yet for node-view-rustdoc.design.1 on 2026-11-17, with one release-path blocker. The owner has
delegated design choices and requested this loop resume. Resolve the findings without changing runtime behavior:

- Choose the normal next patch-release path, currently 0.20.5, including changelog and standard version metadata.
  The supplied AGENTS wording describes the documentation exemption broadly; the release/maintenance procedures
  explicitly describe docs-session upkeep. We do not need to settle or extend that policy to prepare this producer
  change for a normal patch. Actual publication still requires the owner's approval in the build loop. The review's
  Q1 is resolved for design/task preparation by this choice; it is not a release authorization.
- Use the established feature name node-down alerts, distinguishing node-row sampling from explicit Beats handled
  by HeartbeatWatchActor. Retain the no-claim/move/fence authority statement and the existing result description.
- Add an archive handoff to align node-view's stale Purpose with its existing normative observability requirement.
  This changes no behavior and does not ask the builder to edit a handwritten guide or archive the change itself.
- Keep the ADDED rustdoc requirement and human diff inspection. No tautological wording test, new runtime test,
  cross-crate dependency or rustdoc link is needed.

These are narrow corrections following the review; no second review is needed. Design validation does not claim
that cargo doc, implementation tests, the committed gate or a release has run.
