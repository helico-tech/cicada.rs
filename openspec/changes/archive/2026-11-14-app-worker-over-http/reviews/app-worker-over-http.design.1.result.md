Verdict: ready for tasks

Blocking findings: none.

The written design composes the existing Worker lifecycle without weakening lease, completion or cancellation rules. Mode selection precedes configuration, preserves existing store deployments, and explicitly handles incompatible hooks, unavailable startup, registration failure and automatic-stop races. The scope fits four implementation tasks plus finishing.

Notes:
- In design.md, “Composition and client”, include GET /types/{name} in the authentication route coverage. The predecessor's ActorClient::register_contract performs that read after a contract conflict; allowing only the listed GET /types, PUT /types/{name} and lease routes loses the conflict diagnostics to an authorization failure. Include it in the auth/conflict integration test and deployment documentation. This is an implementation/documentation detail, not an unresolved design choice.
- Keep the approximately 90-second statement explicitly about readiness preflight. Worker registers selected contracts afterward; registration is sequential, and a conflict can entail a second request, so it is not a bound for the entire startup.

Questions only the owner can answer: none.

Checked: architecture, relevant ADRs including 0042, all change artifacts, current app specification, adjacent proposals, App parser/configuration/hosting/shutdown code, Worker APIs, and the built predecessor's HTTP adapter in its worktree. OpenSpec list/show and strict validation succeeded.

Not checked: implementation of this proposed change, final landed predecessor revision, runtime tests, workspace suite or live deployment. No repository content was edited other than submitting this review through review.py.
