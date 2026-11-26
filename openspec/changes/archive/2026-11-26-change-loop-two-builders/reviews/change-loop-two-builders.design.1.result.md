Verdict: not yet

## Blocking

1. **Landing order is not decided.** (design.md, Decision 6.) If the second change lands before the first, `status.py` breaks: the change after it now says "after" an archived change, so it and the first change both follow "nothing open", and `order()` reports a problem. Decide: land in order only, or have the session rewrite the follower's `## Order` at landing. Add a test for it.
2. **"First ready change" is wrong for builds already running.** (spec "Two builders…", design Decision 1.) The head is usually `in build` (it is now), and a second change that has started is no longer `ready`. Read literally, the `build:` line drops builds that are running. Base the rule on the busy states (ready, in build, built), and say whether a ready change that shares a crate may be skipped. It can be, and that is what makes item 1 happen.
3. **"Same model family" has no rule.** (design Decision 4, spec.) `--by` and `--built-by` are free text ("Claude Code reviewer subagent", `gpt-6.2-sol`, default `unknown`). Name the families and how a name maps to one. Say what `take` without an id does (skip, not fail) and what happens to design reviews, which have no builder.
4. **The Codex-down path covers only the second change.** (Decision 5.) The first change's review also goes to GPT, so every landing stops while Codex is down. State that on purpose, or give a fallback.
5. **Contradiction with `openspec/config.yaml`.** It says "Throughput is not a goal", and it owns the proposal rules, which the `Crates:` line must join. The design does not name the file. Today no proposal has a `Crates:` line, so nothing pairs until someone adds them. Say who does that.

## Notes

- The proposal says `status.py` "flags two builds that share a crate". The design and spec drop this. The proposal still lists its "To decide" items.
- `gate.py` takes its lock without waiting and fails "busy". Say who retries: the builder, and the session's own gate at landing.
- `delegate_task`'s `target` is an object (`driverKind`, `model`, …), not `codex`. Its worktree must use the branch `feat/<change>`, or `status.py` will not find it.
- Crates that depend on each other (core vs tray) are caught only by the gate on the landed tip. That is acceptable; say so.
- The spec has no scenarios for the missing line or for `Crates: none`.

## Questions for the owner

- None beyond the decisions above.

## Not checked

The builder.md text, `docs/how-to/releasing.md`, or whether the test fixtures can model worktrees.
