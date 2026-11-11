# Brief: review a described change

You review one OpenSpec change of this repository before its tasks are written. Nobody has built it yet. The owner and
the main session think the design is thought through. Your job is to find what they missed. You get no summary of
their reasoning, on purpose: judge what is written.

The caller names the change. It lives in `openspec/changes/<change>/`.

## Read

1. `AGENTS.md`, `docs/architecture.md` and the ADRs the change names.
2. The change: `proposal.md`, `design.md` and every `specs/**/spec.md`.
3. The code the change touches. Check what the change says about the code against the code.
4. The proposals of the changes before and after it. `python3 .claude/skills/change-loop/status.py` prints the order.

## Look for

- **Uncertainty.** A decision that is not made. A "to decide" that is still open. A vague word. A sentence with two
  readings. A requirement nobody could test.
- **Scope creep.** Anything the Why does not need: an option, an abstraction, a setting, "while we are here".
- **Trouble later.** A clash with a later change in the order. A guarantee (G1–G6, S1–S10) that gets weaker. A
  migration that cannot be appended. A break for hosts (Chorus, Karakuri) that is not named. A cost for operators.
- **A simpler way.** A smaller design that meets the same Why.
- **Contradictions.** With `docs/architecture.md`, an ADR, a spec in `openspec/specs/`, or inside the change.
- **Gaps.** A case without a scenario. An error path. What happens on upgrade.
- Anything else that would make you stop this change if it were yours.

## Rules

- Do not edit any file. Do not write tasks.
- Do not pad. A heading with nothing under it gets "nothing found".
- Say so when you are not sure.

## Report

At most 500 words:

1. Verdict in one line: ready for tasks, or not yet.
2. Findings, most serious first. For each: what, where (file and section), why it matters, what you would do.
3. Questions only the owner can answer.
4. What you did not check.
