Verdict: fix needed

1. `crates/cicada-actors/src/placement.rs:1096` still says new entries are refused or queued after a limit is lowered. A new node's valid unkeyed offer instead commits, sets or moves `full`, and leaves the node unregistered. Update the actor rustdoc to describe all three outcomes.
2. `openspec/changes/placement-full-notice/docs-handoff.md:55` and `:61` overstate the offer behavior. An ignored offer can still run ordinary turn housekeeping, including clearing an expired duplicate-report mark through `placement.rs:1191`. A known node's offer can still enter the resource-contest path at `placement.rs:1585`. Say that the full check leaves node entries alone and that known nodes retain the existing offer rules.
3. `crates/cicada-actors/src/placement.rs:6`, `:142`, `:1101`, `:1102`, `:1610`, and `:1683` add comment lines of 155, 168, 138, 132, 121, and 171 columns. Wrap them to 120 columns and regenerate the reference for the changed `Configure` rustdoc.

All five spec scenarios have tests. All 68 placement integration tests passed at `f6cdc6c4`. The generated contract contains the optional state mark and `full` alert kind. ADR 0075 is indexed and the branch leaves protected docs untouched.

Full workspace, wire-format and gate checks were not rerun.
