Verdict: fix needed

1. openspec/config.yaml:43 and .claude/skills/change-loop/SKILL.md:267 require a match task and side-by-side check even for an approved “no board” screen. Define how both checks use the agreed style reference.
2. .claude/skills/change-loop/SKILL.md:149 sends mid-build approval in chat. The builder cannot change the design, and review blocks departures absent from it. Carry the owner's decision into the branch's design before building resumes.
3. .claude/skills/review-loop/code.md:36 can flag system dark colours as a departure. Read SOURCE.md and apply the exception for boards without a dark variant.
4. .claude/skills/change-loop/SKILL.md:264 places the owner's check in Release, as decision 4 requires. Clarify precedence when the same batch contains a GUI change awaiting inspection and a Release: now fix.
5. .claude/skills/change-loop/SKILL.md:240 sends the replacement board but omits updating SOURCE.md. Refresh its version and copy date with the board.

Docs tests passed (11), including contributor Markdown links. Skill tests cover consumer routing and packaging. Skill tests and the full gate were not rerun.
