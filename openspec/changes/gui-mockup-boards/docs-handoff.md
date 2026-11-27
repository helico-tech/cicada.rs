# Docs handoff for gui-mockup-boards

Version: none. This change touches no crate, consumer skill or wire format, so it lands without a version bump,
changelog entry or release, and the installation pins stay as they are.

## What changed

None for users of the library. The way GUI changes are worked on in this repository changes ([MOCK-RULE]): a change
that adds or changes a screen names each screen with its mockup board or "no board", carries copies of its boards in
`mockup/` with `SOURCE.md`, lists its departures (additions included) for the owner to agree, and is built and code
reviewed against the boards in both directions; the change loop reads the live boards again before landing, and a
batch with a GUI change is released only after the owner has seen it next to its boards. The rules live in
`.claude/skills/change-loop/SKILL.md` (GUI changes, Build, Land, Release), `.claude/skills/change-loop/builder.md`,
`.claude/skills/review-loop/design.md` and `code.md`, and `openspec/config.yaml`; they need no prose elsewhere.

## Blocking

None.

## Status

No prose to write. Checks run: `cargo test -p cicada-examples --test skill` (18 passed) and `--test docs` (11 passed),
the script tests (127 passed) and `openspec validate gui-mockup-boards --strict` (valid).
