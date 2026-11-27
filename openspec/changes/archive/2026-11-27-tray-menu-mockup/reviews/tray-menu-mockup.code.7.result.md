Verdict: fix needed

1. The guide handoff still says the menu “shows the error and when,” and task 2 still specifies 200 characters of filtered error text. Update both and the failure scenario to specify the fixed label and time. No error text reaches the row. [Handoff](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/openspec/changes/tray-menu-mockup/docs-handoff.md:66)
2. The Recent Turns handoff calls every reason handler text available in Outcomes. State validation can supply the reason, and rescheduled turns have no terminal outcome. Describe the committed failure or retry reason. [Handoff](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/openspec/changes/tray-menu-mockup/docs-handoff.md:42)

Decision 3 matches the code and the agreed deferral. Recent Turns checks the whole reason before displaying its first line. Cargo tests and live AppKit rendering were not run.
