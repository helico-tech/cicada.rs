#!/usr/bin/env python3
"""Review requests of the change loop (ADR 0055, from ADR 0030), as files that any agent harness can read and write.

The change loop asks for a review and starts a reviewer subagent that takes it by its id; another harness can take
open requests through the review loop.

The files live in `.reviews/` in the main checkout, which git ignores. A review is named `<change>.<kind>.<round>`,
with the kind `design` or `code`. Its state is which of its files exist:

    <id>.md          the request: open
    <id>.taken       who took it, and when: taken
    <id>.result.md   the report: done

    review.py request design|code <change> [--head REF] [--built-by NAME] [--note-file FILE]
                                                         ask for a review; prints its id
    review.py take [ID] [--wait] [--timeout SECONDS] [--by NAME] [--same-family REASON]
                                                         take request ID, or the oldest open one it may; prints it
    review.py done <id> <file>                           hand in the report of a taken review
    review.py wait <id> [--timeout SECONDS]              wait for the report; prints it
    review.py list                                       every review, its state, builder and reviewer

`take` and `wait` end with status 1 when nothing came within the timeout; without one they wait as long as it takes.
`take ID` claims exactly that request and fails when it is missing, taken or done; it does not wait.

A code review is done by a model other than its builder's (ADR 0073). `request code` needs `--built-by` and `take` of a
code review needs `--by`, each a name that starts with its family: `claude`, or `gpt` (`codex` and `openai` count as
`gpt`). `take ID` refuses a reviewer of the builder's family, and `take` without an id skips such requests, unless
`--same-family REASON` says why no other family can review (Codex unavailable); the reason is recorded in the taken
marker and at the end of the report. Design reviews have no builder: any reviewer may take them.
"""

import argparse
import datetime
import os
import pathlib
import re
import subprocess
import sys
import time

POLL = 5
NAME = re.compile(r"^(?P<change>[a-z0-9-]+)\.(?P<kind>design|code)\.(?P<round>[0-9]+)$")
FAMILIES = {"claude": "claude", "gpt": "gpt", "codex": "gpt", "openai": "gpt"}
BRIEF = {"design": ".claude/skills/review-loop/design.md", "code": ".claude/skills/review-loop/code.md"}


def git(*args):
    here = pathlib.Path(__file__).resolve().parent
    return subprocess.run(["git", *args], cwd=here, capture_output=True, text=True, check=True).stdout.strip()


# The main checkout, also when this runs from a worktree.
ROOT = pathlib.Path(git("rev-parse", "--path-format=absolute", "--git-common-dir")).parent
DIR = ROOT / ".reviews"


def family(name):
    """The model family a name starts with, `claude` or `gpt`; None for a name without one."""
    lowered = (name or "").lower()
    return next((fam for prefix, fam in FAMILIES.items() if lowered.startswith(prefix)), None)


def now():
    return datetime.datetime.now().astimezone().isoformat(timespec="seconds")


def fields(text):
    """The `key: value` lines of a file's front matter."""
    match = re.match(r"---\n(.*?)\n---\n", text, re.S)
    return dict(line.split(": ", 1) for line in match.group(1).splitlines() if ": " in line) if match else {}


def requests():
    """Every request, oldest first, as (id, path)."""
    found = [(p.stem, p) for p in DIR.glob("*.md") if NAME.match(p.stem)]
    return sorted(found, key=lambda item: (item[1].stat().st_mtime, item[0]))


def state(review):
    if (DIR / f"{review}.result.md").exists():
        return "done"
    return "taken" if (DIR / f"{review}.taken").exists() else "open"


def write(path, text):
    """Writes so that a reader sees the whole file or none of it."""
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(text)
    os.replace(temporary, path)


def request(args):
    if not (ROOT / "openspec" / "changes" / args.change).is_dir():
        sys.exit(f"review: no open change {args.change}")
    if args.kind == "code" and not args.head:
        sys.exit("review: a code review needs --head")
    if args.kind == "code" and family(args.built_by) is None:
        sys.exit("review: a code review needs --built-by with a name that starts with claude or gpt")
    DIR.mkdir(exist_ok=True)
    names = (NAME.match(review) for review, _ in requests())
    rounds = [int(m["round"]) for m in names if m["change"] == args.change and m["kind"] == args.kind]
    number = max(rounds, default=0) + 1
    review = f"{args.change}.{args.kind}.{number}"
    lines = [f"id: {review}", f"kind: {args.kind}", f"change: {args.change}", f"round: {number}"]
    if args.kind == "code":
        head = git("rev-parse", "--verify", f"{args.head}^{{commit}}")
        lines += [f"base: {git('merge-base', 'main', head)}", f"head: {head}", f"built-by: {args.built_by}"]
        if number > 1:
            lines.append(f"previous: {fields((DIR / f'{args.change}.code.{number - 1}.md').read_text())['head']}")
    lines.append(f"requested: {now()}")
    note = pathlib.Path(args.note_file).read_text().strip() if args.note_file else ""
    body = f"Follow `{BRIEF[args.kind]}`, and hand in the report with `python3 scripts/review.py done {review} <file>`.\n"
    if note:
        body += f"\n## Note from the requester\n\n{note}\n"
    write(DIR / f"{review}.md", "---\n" + "\n".join(lines) + "\n---\n\n" + body)
    print(review)


def until(found, timeout):
    """Calls `found` every few seconds until it returns something, or the timeout passes."""
    deadline = None if timeout is None else time.monotonic() + timeout
    while True:
        result = found()
        if result is not None or (deadline is not None and time.monotonic() >= deadline):
            return result
        time.sleep(POLL)


def refusal(review, by, same_family):
    """(why `by` may not take the request, or None when it may; whether taking it is a same-family review)."""
    request = fields((DIR / f"{review}.md").read_text())
    if request.get("kind") != "code":
        return None, False
    if family(by) is None:
        return "a code review needs --by with a name that starts with claude or gpt", False
    builder = request.get("built-by")
    if family(builder) != family(by):
        return None, False
    if same_family:
        return None, True
    return f"built by {builder}; a reviewer of another family takes it, or pass --same-family REASON", False


def claim(review, by, same_family=None):
    """Marks an open request as taken; False when another reviewer has it, it is done, or `by` may not take it."""
    if state(review) != "open":
        return False
    reason, same = refusal(review, by, same_family)
    if reason:
        return False
    try:
        handle = os.open(DIR / f"{review}.taken", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
    except FileExistsError:
        return False
    with os.fdopen(handle, "w") as taken:
        taken.write(f"by: {by or 'unknown'}\ntaken: {now()}\n" + (f"same-family: {same_family}\n" if same else ""))
    return True


def take(args):
    DIR.mkdir(exist_ok=True)
    if args.id is not None:
        path = DIR / f"{args.id}.md"
        if not NAME.match(args.id) or not path.exists():
            sys.exit(f"review: no request {args.id}")
        if state(args.id) == "open":
            reason, _ = refusal(args.id, args.by, args.same_family)
            if reason:
                sys.exit(f"review: {args.id}: {reason}")
        if not claim(args.id, args.by, args.same_family):
            sys.exit(f"review: {args.id} is {state(args.id)}, not open")
        got = args.id, path
    else:

        def oldest():
            return next(((r, path) for r, path in requests() if claim(r, args.by, args.same_family)), None)

        got = until(oldest, args.timeout) if args.wait else oldest()
    if got is None:
        print("review: no open request")
        return 1
    review, path = got
    print(f"Took {review} ({path}).\n")
    print(path.read_text())
    return 0


def taken_fields(review):
    taken = DIR / f"{review}.taken"
    return fields("---\n" + taken.read_text() + "---\n") if taken.exists() else {}


def done(args):
    if not (DIR / f"{args.id}.md").exists():
        sys.exit(f"review: no request {args.id}")
    if state(args.id) != "taken":
        sys.exit(f"review: {args.id} is {state(args.id)}, not taken")
    report = pathlib.Path(args.file).read_text()
    taken = taken_fields(args.id)
    if "same-family" in taken:
        report = report.rstrip("\n") + f"\n\nSame-family review by {taken['by']}: {taken['same-family']}\n"
    write(DIR / f"{args.id}.result.md", report)
    print(f"handed in {args.id}")


def wait(args):
    if not (DIR / f"{args.id}.md").exists():
        sys.exit(f"review: no request {args.id}")
    result = DIR / f"{args.id}.result.md"
    if until(lambda: True if result.exists() else None, args.timeout) is None:
        print(f"review: no report for {args.id} yet ({state(args.id)})")
        return 1
    print(result.read_text())
    return 0


def listing(_args):
    rows = []
    for review, path in requests():
        row = [review, state(review)]
        builder = fields(path.read_text()).get("built-by")
        if builder:
            row.append(f"built by {builder}")
        taken = taken_fields(review)
        if "by" in taken:
            row.append(f"taken by {taken['by']}")
        if "same-family" in taken:
            row.append(f"same family: {taken['same-family']}")
        result = DIR / f"{review}.result.md"
        verdict = next((line for line in result.read_text().splitlines() if line.strip()), "") if result.exists() else ""
        rows.append(row + ([verdict] if verdict else []))
    if not rows:
        print("no reviews")
    for row in rows:
        print("  ".join(row))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("request")
    p.add_argument("kind", choices=["design", "code"])
    p.add_argument("change")
    p.add_argument("--head")
    p.add_argument("--built-by")
    p.add_argument("--note-file")
    p.set_defaults(run=request)
    p = commands.add_parser("take")
    p.add_argument("id", nargs="?")
    p.add_argument("--wait", action="store_true")
    p.add_argument("--timeout", type=float)
    p.add_argument("--by")
    p.add_argument("--same-family", metavar="REASON")
    p.set_defaults(run=take)
    p = commands.add_parser("done")
    p.add_argument("id")
    p.add_argument("file")
    p.set_defaults(run=done)
    p = commands.add_parser("wait")
    p.add_argument("id")
    p.add_argument("--timeout", type=float)
    p.set_defaults(run=wait)
    p = commands.add_parser("list")
    p.set_defaults(run=listing)
    args = parser.parse_args()
    return args.run(args) or 0


if __name__ == "__main__":
    sys.exit(main())
