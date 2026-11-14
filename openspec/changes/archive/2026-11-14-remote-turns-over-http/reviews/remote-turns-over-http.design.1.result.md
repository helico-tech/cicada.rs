Verdict: ready for tasks

Blocking findings: none.

The design preserves epoch fencing, server-derived completion policy, the permanent Rejected-to-Failed transition, and abandonment without release after handler end. Distinguishing core alternatives from HTTP status errors avoids treating proxy failures as authoritative lease decisions. The separate strict execution message view addresses the existing management view's deliberately lenient policies and default counters. The transport remains one coherent change with five implementation tasks.

Notes:
- In design.md, “Routes and trust,” narrow “No server observer” to completion and any additional HTTP-layer observation. Core lease admission intentionally reports confirmed claim-limit failures to server observers (architecture.md, “Leases outside the node”; runtime/system.rs, fail_at_claim_limit). The surrounding requirement to preserve core behavior already supplies the intended rule; make the shorthand equally clear.
- When writing the wire task, explicitly check generated OpenAPI required lists against the nullable-but-required execution fields. Ordinary Option derives would accept omission, and the existing MessageView is unsuitable for this contract; the planned strict decoding scenarios should cover both schema and decoder behavior.

Questions for the owner: none.

Checked: all change artifacts and ADR 0040; relevant architecture, predecessor designs/ADRs, adjacent proposals, current lease/client/router/parser/model/conversion code, and OpenSpec list/show/strict validation (passed).

Not checked: implementation of this change or the unlanded Worker/completion predecessors, live HTTP/proxy behavior, or runtime tests. No test suite was run.
