Verdict: fix needed

1. `openspec/changes/remove-inspector/.openspec.yaml:1` is missing `retire_capabilities: true`. Design decision 6 retires the entire inspector capability. In a scratch copy of ad1c85aa, `openspec archive remove-inspector -y` aborted with “Spec must have at least one requirement” when the delta removed all 16 requirements. Strict change validation alone passes. Adding the setting made the archive succeed, delete the inspector spec, and apply the change-loop scenario update.

Notes

- `openspec/changes/worker-view/proposal.md:30` still names an inspector view as follow-up work. This proposal predates the removal and is outside this change.
- The full gate record for ad1c85aa is a pass. All six documentation files in the handoff match the owner's commit cf6f5079.

What I did not check

- I used the recorded suite results and did not rerun the suite or E2E. I reviewed the tray's menu order in source. Live AppKit inspection remains a release step.
