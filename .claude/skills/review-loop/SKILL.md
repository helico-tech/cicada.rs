---
name: review-loop
description: Start the review loop of this repository (ADR 0030) - wait for review requests from the design loop and the build loop, review each one, and hand in a short report. Written for any agent harness, not only Claude Code.
disable-model-invocation: true
---

# The review loop

The design loop and the build loop ask for reviews by writing request files; you take them, review, and hand in a
short report (ADR 0030). This file is for any agent harness that can read files and run a shell with `git` and
`python3`. Work from the main checkout of the repository.

## Loop

1. **Take** a request:

   ```sh
   python3 scripts/review.py take --wait --by "<your harness and model>"
   ```

   It waits until a request is open, claims it so that no other reviewer takes it, and prints it. Without `--timeout`
   it waits as long as it takes. If your harness stops long shell commands, add `--timeout <seconds>` below that limit
   and run it again when it ends with status 1.
2. **Review** it by the brief the request names: `design.md` or `code.md` in this folder. Each brief ends with how to
   hand in the report (`python3 scripts/review.py done <id> <file>`).
3. Go to 1.

Each review stands alone: judge what is written, not what an earlier review said. If your harness can, give each
review a fresh context. In Claude Code: run the `take` in the background, and give each request to a subagent
(`general-purpose`, no isolation) with the prompt "Read `.claude/skills/review-loop/<kind>.md` and follow it. Request:
`.reviews/<id>.md`."

## Rules

- Do not edit, commit, push, merge or tag anything in the repository. Only `scripts/review.py` writes in `.reviews/`.
- A check you did not run is "not run", never "passed". Say so when you are not sure.
- Keep it short and concrete: the owner wants quick rounds.
