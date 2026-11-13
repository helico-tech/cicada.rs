#!/usr/bin/env python3
"""Review requests between the loops of this repository (ADR 0030), as files that any agent harness can read and write.

The files live in `.reviews/` in the main checkout, which git ignores. A review is named `<change>.<kind>.<round>`,
with the kind `design` or `code`. Its state is which of its files exist:

    <id>.md          the request: open
    <id>.taken       who took it, and when: taken
    <id>.result.md   the report: done

    review.py request design|code <change> [--head REF] [--note-file FILE]   ask for a review; prints its id
    review.py take [--wait] [--timeout SECONDS] [--by NAME]                  take the oldest open request; prints it
    review.py done <id> <file>                                               hand in the report of a taken review
    review.py wait <id> [--timeout SECONDS]                                  wait for the report; prints it
    review.py list                                                           every review and its state

`take` and `wait` end with status 1 when nothing came within the timeout; without one they wait as long as it takes.
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
BRIEF = {"design": ".claude/skills/review-loop/design.md", "code": ".claude/skills/review-loop/code.md"}


def git(*args):
    here = pathlib.Path(__file__).resolve().parent
    return subprocess.run(["git", *args], cwd=here, capture_output=True, text=True, check=True).stdout.strip()


# The main checkout, also when this runs from a worktree.
ROOT = pathlib.Path(git("rev-parse", "--path-format=absolute", "--git-common-dir")).parent
DIR = ROOT / ".reviews"


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
    DIR.mkdir(exist_ok=True)
    names = (NAME.match(review) for review, _ in requests())
    rounds = [int(m["round"]) for m in names if m["change"] == args.change and m["kind"] == args.kind]
    number = max(rounds, default=0) + 1
    review = f"{args.change}.{args.kind}.{number}"
    lines = [f"id: {review}", f"kind: {args.kind}", f"change: {args.change}", f"round: {number}"]
    if args.kind == "code":
        head = git("rev-parse", "--verify", f"{args.head}^{{commit}}")
        lines += [f"base: {git('merge-base', 'main', head)}", f"head: {head}"]
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


def take(args):
    def claim():
        for review, path in requests():
            if state(review) != "open":
                continue
            try:
                handle = os.open(DIR / f"{review}.taken", os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            except FileExistsError:
                continue
            with os.fdopen(handle, "w") as taken:
                taken.write(f"by: {args.by}\ntaken: {now()}\n")
            return review, path
        return None

    DIR.mkdir(exist_ok=True)
    got = until(claim, args.timeout) if args.wait else claim()
    if got is None:
        print("review: no open request")
        return 1
    review, path = got
    print(f"Took {review} ({path}).\n")
    print(path.read_text())
    return 0


def done(args):
    if not (DIR / f"{args.id}.md").exists():
        sys.exit(f"review: no request {args.id}")
    if state(args.id) != "taken":
        sys.exit(f"review: {args.id} is {state(args.id)}, not taken")
    write(DIR / f"{args.id}.result.md", pathlib.Path(args.file).read_text())
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
    for review, _ in requests():
        taken = DIR / f"{review}.taken"
        by = fields("---\n" + taken.read_text() + "---\n").get("by", "") if taken.exists() else ""
        result = DIR / f"{review}.result.md"
        verdict = next((line for line in result.read_text().splitlines() if line.strip()), "") if result.exists() else ""
        rows.append((review, state(review), by, verdict))
    if not rows:
        print("no reviews")
    for row in rows:
        print("  ".join(row).rstrip())


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("request")
    p.add_argument("kind", choices=["design", "code"])
    p.add_argument("change")
    p.add_argument("--head")
    p.add_argument("--note-file")
    p.set_defaults(run=request)
    p = commands.add_parser("take")
    p.add_argument("--wait", action="store_true")
    p.add_argument("--timeout", type=float)
    p.add_argument("--by", default="unknown")
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
