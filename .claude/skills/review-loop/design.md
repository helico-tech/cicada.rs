# Brief: review a described change

The change loop asks you to review one OpenSpec change before its tasks are written. Nobody has built it yet. The
owner and the change loop think the design is thought through; find what they missed. You get no summary of their
reasoning, on purpose: judge what is written. The request (`.reviews/<id>.md`) names the change. It lives in
`openspec/changes/<change>/` of the main checkout, and may not be committed yet.

## Read

1. `AGENTS.md`, the parts of `docs/architecture.md` that the change touches, and the ADRs it names.
2. The change: `proposal.md`, `design.md`, every `specs/**/spec.md`, and the boards in `mockup/` if it has any.
3. The code the change touches. Check what the change says about the code against the code.
4. The proposals of the changes before and after it. `python3 scripts/status.py` prints the order.

## Look for

- **Size.** More than one behaviour, or more than 5 tasks of work. Say how you would split it.
- **Uncertainty.** A decision that is not made, a vague word, a sentence with two readings, a requirement nobody could
  test.
- **Scope creep.** Anything the Why does not need: an option, an abstraction, a setting, "while we are here".
- **Trouble later.** A clash with a later change in the order. A guarantee (G1–G6, S1–S10) that gets weaker. A
  migration that cannot be appended. A break for hosts that is not named.
- **A simpler way.** A smaller design that meets the same Why.
- **Contradictions.** With `docs/architecture.md`, an ADR, a spec in `openspec/specs/`, or inside the change.
- **Gaps.** An error path or an upgrade without a scenario.
- **Docs.** A doc block, link, guarantee row or skill route the change breaks and does not name. The builder may not
  edit `README.md`, `AGENTS.md` or the handwritten docs: the prose goes into a docs handoff.
- **Boards.** A change that adds or changes a screen: each such screen is named with its board in `mockup/` (with
  `mockup/SOURCE.md`) or with an agreed "no board" and the board whose style it follows. Every element of a screen's
  own board (sections, columns and their order, controls, texts, states) is in the design or under `Departures from
  the board` with a reason, and so is everything the design adds to a screen beyond its board. A "no board" screen's
  controls and sections are described in the design; its style board sets only its style.

## What blocks

Only what must be settled before the tasks are written: an open decision, a contradiction, a weaker guarantee, an
error path without a scenario, a change that is too big, a gap in Boards. Everything else is a note, at most five.

## Report

At most 300 words. Write it to a file outside the repository and hand it in with
`python3 scripts/review.py done <id> <file>`.

1. The first line: `Verdict: ready for tasks` or `Verdict: not yet`.
2. Blocking findings, most serious first. For each: what, where (file and section), why it matters, what you would do.
3. Notes.
4. Questions only the owner can answer.
5. What you did not check.
