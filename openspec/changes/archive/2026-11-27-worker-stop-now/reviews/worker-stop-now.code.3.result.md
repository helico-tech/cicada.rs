Verdict: fix needed

1. `CHANGELOG.md:25`, `design.md:10` and `docs-handoff.md:8,21,56` generalize lease-release attempts. Stop Now during post-handler result validation skips release (`executor.rs:497–504,723–746`). Qualify the attempt as applying when cancellation stops work before the handler ends and the lease is still held.
2. `docs-handoff.md:68` says “without a commit.” A completion can apply without acknowledgement (`executor.rs:912–943`), and dropping the tray record still produces Abandoned (`turns.rs:129–137`). Use “without an acknowledged commit,” as in the tray rustdoc and spec.

The changelog diff touches only the two 0.26.0 bullets. Diff check passed. Tests were not run.
