Verdict: pass

## Blocking findings

None.

## Gap-fills (judged)

- "No new places" only for marks with a `host`: accepted. The spec and ADR 0071 scope it to the reported mark. Blocking instance-only marks would change ADR 0069 behaviour without an ADR.
- A `Duplicate` for a silent node changes nothing: accepted. It matches the decode rule (a conflict only while live), and a silent node gets no places anyway.
- A report-only mark clears at the first pool turn after its silence time: accepted. `free_places` and `grant_waiting` only run inside turns, and `end_reports` runs at the start of each one, so placement never sees a stale mark. An Ask is itself a turn.
- `NodeConfig::hostname` bounds like `label`: accepted, and tested (`a_blank_a_nul_or_a_too_long_hostname_fails_start`).

## Notes

1. `member.rs` `lock()`: the lock file is opened with `create + write`. A read-only identity folder (for example a pre-made `placement-identity.json` on a read-only mount) loaded before this change. Now it fails with `Invalid` ("cannot be opened"). The design only covers `try_lock` errors. Consider treating a read-only open failure like "no locks": warn and go on. At least name it in the upgrade note.
2. `PlacementMember::logged` is never reset. A second duplicate episode with the same host (other host → own → same other host) is not logged again. The rustdoc's "once for each other host name in a row" is ambiguous here. Either reset `logged` on a clean round, or say "once per other host name".
3. Between turns, a pool's state read (operator view) can still show a report-only mark past its end. The handoff's "at the first turn" covers this. Worth one line in the guide.
4. CHANGELOG and handoff are accurate. Versions are 0.25.0 in `Cargo.toml`, both skills, `plugin.json` and OpenAPI. The contract reference is regenerated. No protected docs are edited.

## What I did not check

I ran no tests; I relied on the builder's gate evidence for 673d9dfd plus the docs pass. I did not check the lock on Windows or NFS. I did not check the remote-worker (HTTP `TurnSource`) path for `ActorSystem::node`.
