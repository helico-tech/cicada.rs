Verdict: not yet

## Blocking

1. **The round model contradicts its test** (design, decision 3 and Tests). With at most 8 reads at once, eight slow containers do hold up a ninth. The design also does not say whether a round waits for its slowest read. If it does, one chatty log delays every container. Decide: a container still being read is skipped, and rounds do not wait. Then fix the test.
2. **Error paths are missing** (decision 4, spec). Nothing covers a failed `list`, or `logs_since` kinds such as `InvalidSpec` and future ones (`non_exhaustive`). A page size outside 1 to `LOG_PAGE_MAX` is not checked. The spec has a scenario only for `DaemonUnavailable`. Add scenarios for `Unsupported`, a gone container and a failed `list`, and retry every other kind with the backoff.
3. **The last lines are lost on a quick `Remove`.** The actor removes at once (`actor/mod.rs`, `remove`). Lines written after the last round are lost until `container-output-collect`, yet the spec promises "every line". Put this limit in the spec as a scenario and in the handoff.

## Notes

- `App::task` needs `anyhow::Result<()>`, but `run` gives `()`, so the example does not compile.
- Decision 5 sets a rule for retention (remove the container before its outputs). Add it to retention's proposal.
- Name the settings API. Say whether the 8 is a setting. Replace "up to 1 minute" with a formula.
- Skip empty pages whose `next` equals the cursor. Prune the drained set to the names `list` returned.
- A runtime without `logs_since` gets every container marked drained silently. Log it once.

## Questions for the owner

- Q1: Should every node run an archiver? Each extra one repeats the Docker log scans. My pick: one per daemon, stated in the handoff.

## Not checked

The code of app-tasks and container-output-store (neither is built), Docker's label filter, and whether it compiles.
