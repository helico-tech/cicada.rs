Verdict: fix needed

1. `CHANGELOG.md:25` and `docs-handoff.md:8,21,56–57` still promise a release attempt whenever work stops before its handler ends. `executor.rs:493–504` skips release when lease loss is known or an operator cancellation won first. Qualify the attempt accordingly.
2. `proposal.md:12–13` still says the leases go back to the server. Describe the conditional release attempt.

The tray handoff correctly uses “without an acknowledged commit.” Diff check passed. Tests were not run.
