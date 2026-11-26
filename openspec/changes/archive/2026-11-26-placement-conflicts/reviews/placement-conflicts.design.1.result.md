Verdict: not yet

## Blocking

1. **No ADR for a changed rule and wire format.** ADR 0067 (accepted) says an offer "replaces the node's last offer" and names the notices `silent` and `back`. This change adds an exception, a node field and a notice kind. AGENTS.md: a wire-format change needs an ADR. Being inside the unreleased line does not remove that. Fix: a short ADR that extends 0067 (the conflict rule, the field, the kind, the limits), plus its row in `docs/adr/README.md`.
2. **The mark's shape and end are only in the design.** The spec says "one `conflict` alert while the mark lasts" but never says when the mark ends. Its only scenario covers the end by silence; there is no scenario for "the second machine stops for one silence time". The field's shape is also unsettled: Decision 1 says `conflict` is the other instance, Decision 5 says it also holds the time of the last conflicting offer. Fix: give the shape (for example `conflict: {instance, at}`, with `at` renewed by each conflicting offer). Put both end rules in the requirement, and add a scenario where the second machine stops, the mark clears, and its return sends a new alert.

## Notes

- Until `placement-member` lands, `instance` is still "drawn at each process start" (`Offer` rustdoc; guide line 572). So in that window a restart inside the silence time is a false conflict, ignored for up to one silence time. This is fine inside one release, but say so.
- "Ignored offers are not counted" is vague. Say what is not counted.
- Decoded-state check: `conflict` only on a live node, with an instance other than `instance`. A third instance while marked: say whether it replaces the recorded one.
- Docs handoff: `docs/architecture.md` line 443 lists `silent`/`back`. Name it next to the guide.
- The notice carries no instance, so the operator must read the state to see which one.

## Questions

- CF-Q1: Should the `conflict` notice carry the other instance? My pick: no, the state has it.

## Not checked

The contract snapshot and sample regeneration, the guide doctests, `placement-holders` code in its worktree.
