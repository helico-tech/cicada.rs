Verdict: fix needed

1. `openspec/changes/placement-full-notice/docs-handoff.md:54` still says “nothing else changes,” contradicting its new housekeeping note. The handler calls `end_reports` before processing the offer and can clear an expired conflict mark (`placement.rs:1192`). Say the full-pool check leaves node entries alone.
2. The proposed text at `docs-handoff.md:73` still says a known node can “always renew.” The existing instance-conflict and resource-contest paths can both bypass renewal (`placement.rs:1576–1589`). Say known nodes keep their existing offer rules.

The lower-limit rustdoc is corrected. Rewraps preserved wording, all changed Rust lines fit 120 columns, and the ADR index contains 0074 and 0075. All 31 wire-format tests passed, including the regenerated contract check. The workspace suite and landing gate were not run.
