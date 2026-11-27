Verdict: fix needed

1. .claude/skills/review-loop/code.md:36 still requires an exact comparison against every copied board. An approved no-board sheet can follow board Y's style while using different controls. The brief would flag those controls as departures. Carry the style-only check into the review briefs so that exact comparison applies to a screen's own board.

The four other fixes are in place. The builder commits the owner's departure entry verbatim, the code review honors the dark-colour exception, the owner's inspection takes precedence over Release: now, and a replacement board travels with refreshed SOURCE.md.

The diff passed git diff --check. Tests were not rerun.
