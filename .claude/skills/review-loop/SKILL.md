---
name: review-loop
description: Start the review loop of this repository (ADR 0055) - take review requests of the change loop in another agent harness, review each one, and hand in a short report. The change loop starts its own reviewer subagents; this loop is for any other harness.
disable-model-invocation: true
---

# The review loop

The change loop asks for reviews by writing request files (ADR 0055). It starts its own reviewer subagent for each
request, which takes it by id (`python3 scripts/review.py take <id>`). This loop is for another agent harness, when the
owner wants a request reviewed there: you take requests, review, and hand in a short report. It works in any harness
that can read files and run a shell with `git` and `python3`. Work from the main checkout of the repository.

## Loop

1. **Take** a request:

   ```sh
   python3 scripts/review.py take --wait --by "<your model>"
   ```

   The name starts with your model's family, `claude` or `gpt` (`codex` and `openai` count as `gpt`), for example
   `gpt-6.2-sol in Codex`: a code review needs it, and is never taken by the family that built the change (ADR 0073).

   It waits until a request is open, claims it so that no other reviewer takes it, and prints it. Without `--timeout`
   it waits as long as it takes. If your harness stops long shell commands, add `--timeout <seconds>` below that limit
   and run it again when it ends with status 1. When the owner gives you an id, take exactly that one:
   `python3 scripts/review.py take <id> --by "<your model>"`.
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
