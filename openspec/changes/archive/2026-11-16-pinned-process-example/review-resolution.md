# Design review resolution

Opus 5.5 reviewed pinned-process-example.design.1 on 2026-11-16: not yet, with two delegated design choices blocking.
The report exceeded its 300-word brief; retain it unchanged as submitted rather than rewriting independent evidence.

- NeedsReconciliation is absorbing, including late terminal observations from the original host. This is deliberately
  conservative when two host processes share an id: a new host may mark a live old run uncertain, but cannot act on it.
- Resolve the otherwise permanent storage dead end by allowing existing-API deletion after independent external
  verification/cleanup. Deletion does not perform or prove cleanup, and replacement requires a new explicit key.
  Add tests for the late-observation ordering and this manual disposition path. No Abandon protocol is needed.
- Specify shutdown: close host admission, stop/reap and flush observations within five seconds while system/pump/store
  remain open; then bounded system stop, pump join and store close. Unconfirmed cleanup or flush is reported.
- Retain the Stop-before-Start tombstone: its extra slot cost preserves stop intent across an uncommitted outside
  action. Test the lost-Stop-commit case, not only the ordinary durable Stop path.
- Keep the separate OS helper test so actor/runtime tests retain virtual time. Host construction chooses the child
  executable; Linux/macOS run OS tests, other platforms get no implied OS verification. No new gate check is added.
- Align proposal with ADR 0053 and require landed guide/skill predecessors before using their real consumer routes.

The corrections settle the reviewed choices without replacing the example protocol; no second review is needed.
Tasks and ready approval use the owner's delegation. Strict validation/docs checks do not substitute for the later
lost-commit and OS tests, which remain implementation evidence to obtain.
