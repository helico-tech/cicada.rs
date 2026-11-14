Verdict: ready for tasks

Blocking findings: none.

The transition fits the existing Turn cancellation/state-write boundary. Status dispatch before normalization prevents replay after cancellation; preserving the open main wait and replies remains compatible with ADR 0044’s later cleanup overlap. Mailbox-order races, final-step completion, terminal no-ops, unavailable cancelling behavior and mixed-version rollout are explicit. The change is one behavior within the stated task budget.

Notes:
- In specs/flow-cancellation/spec.md, “Atomic failure”, distinguish handler retry policy from commit/lease recovery. An unapplied commit after the handler returned leaves pending work subject to ordinary lease reclamation and claim limits; it does not itself invoke sender-selected handler retry. The design already delegates to ordinary core rules, so this is wording/test precision, not a new decision. Also test an unknown commit acknowledgement by rereading/retrying idempotently, without assuming the transaction rolled back.
- Strict OpenSpec validation passes. Its informational archive warning for flow-sends is expected until the ordered predecessor creates that spec; archive predecessors first.

Questions for the owner: none.

Checked architecture, ADRs 0031/0044 and relevant core control semantics, all cancellation artifacts, predecessor designs and adjacent proposals, current flow code, Turn::cancel/state_mut and cancellation application in all three backends. Current source still implements only Continue/running/done; the reserved APIs and reply/wait behavior are correctly treated as predecessor work, not existing implementation.

Not checked: implementation or generated contracts after the four unbuilt predecessors land. No builds, runtime tests or workspace suite were run.
