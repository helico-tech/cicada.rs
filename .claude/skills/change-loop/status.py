#!/usr/bin/env python3
"""Prints the open OpenSpec changes in their order, the state of each, and what the change loop does next.

The order comes from the `## Order` section of each proposal ("After `x`", "before `y`"). The state comes from the
files of the change: only a proposal, described (design or spec), ready (a `tasks.md`), in build (some tasks ticked),
built (all ticked). While a change is being built, its `tasks.md` is read from its worktree.
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
CHANGES = ROOT / "openspec" / "changes"
TASK = re.compile(r"^\s*- \[( |x|X)\]", re.M)


def worktrees():
    """Branch name -> worktree path, for every worktree of the repository."""
    out = subprocess.run(
        ["git", "-C", str(ROOT), "worktree", "list", "--porcelain"], capture_output=True, text=True, check=True
    ).stdout
    found, path = {}, None
    for line in out.splitlines():
        if line.startswith("worktree "):
            path = pathlib.Path(line[len("worktree ") :])
        elif line.startswith("branch refs/heads/") and path is not None:
            found[line[len("branch refs/heads/") :]] = path
    return found


def order_of(proposal):
    """(after, before) named in the first paragraph of the Order section; None where it names none."""
    match = re.search(r"^## Order\n+(.*?)(?=\n\n|\n## |\Z)", proposal, re.S | re.M)
    if not match:
        return None
    text = match.group(1).replace("\n", " ")
    after = re.search(r"[Aa]fter `([a-z0-9-]+)`", text)
    before = re.search(r"[Bb]efore `([a-z0-9-]+)`", text)
    return (after.group(1) if after else None, before.group(1) if before else None)


def read(name, trees):
    folder = CHANGES / name
    tree = trees.get(f"feat/{name}")
    built_in = tree / "openspec" / "changes" / name if tree else folder
    tasks_file = built_in / "tasks.md" if (built_in / "tasks.md").exists() else folder / "tasks.md"
    done = total = 0
    if tasks_file.exists():
        marks = TASK.findall(tasks_file.read_text())
        total, done = len(marks), sum(1 for mark in marks if mark != " ")
    described = (folder / "design.md").exists() or any(folder.glob("specs/**/spec.md"))
    if not tasks_file.exists():
        state = "described" if described else "proposal"
    elif done == 0:
        state = "ready"
    elif done < total:
        state = "in build"
    else:
        state = "built"
    proposal = folder / "proposal.md"
    return {
        "name": name,
        "state": state,
        "tasks": f"{done}/{total}" if tasks_file.exists() else "-",
        "tree": str(tree.relative_to(ROOT)) if tree and tree.is_relative_to(ROOT) else (str(tree) if tree else "-"),
        "order": order_of(proposal.read_text()) if proposal.exists() else None,
    }


def main():
    trees = worktrees()
    names = sorted(d.name for d in CHANGES.iterdir() if d.is_dir() and d.name != "archive")
    changes = {name: read(name, trees) for name in names}
    problems = []

    # The queue: start at the change whose predecessor is not open (none, or archived), then follow "after".
    followers = {}
    for change in changes.values():
        if change["order"] is None:
            continue
        after = change["order"][0]
        key = after if after in changes else None
        followers.setdefault(key, []).append(change["name"])
    queue, current = [], None
    while current in followers:
        nexts = followers[current]
        if len(nexts) > 1:
            problems.append(f"{' and '.join(nexts)} both come after {current or 'nothing open'}; the order must be one line")
        current = nexts[0]
        if current in queue:
            problems.append(f"the order loops at {current}")
            break
        queue.append(current)
    for name in names:
        if name not in queue:
            why = "has no Order section" if changes[name]["order"] is None else "is not reached by the order"
            problems.append(f"{name} {why}")
    for earlier, later in zip(queue, queue[1:]):
        before = changes[earlier]["order"][1]
        if before is not None and before != later:
            problems.append(f"{earlier} says it comes before {before}, but {later} comes after it")

    width = max((len(name) for name in names), default=6)
    print(f"{'#':>2}  {'change':<{width}}  {'state':<9}  {'tasks':<5}  worktree")
    for number, name in enumerate(queue, 1):
        c = changes[name]
        print(f"{number:>2}  {name:<{width}}  {c['state']:<9}  {c['tasks']:<5}  {c['tree']}")
    print()

    if not queue:
        print("build:    nothing; no open change has a place in the order")
        print("describe: nothing")
    else:
        head = changes[queue[0]]
        if head["state"] in ("ready", "in build", "built"):
            tree = f"worktree {head['tree']}" if head["tree"] != "-" else "no worktree yet"
            print(f"build:    {head['name']} ({head['state']}, tasks {head['tasks']}, {tree})")
            if len(queue) == 1:
                print("describe: nothing; no change is left after it")
            else:
                ahead = changes[queue[1]]
                if ahead["tasks"] == "-":
                    print(f"describe: {ahead['name']} ({ahead['state']})")
                else:
                    print(f"describe: nothing; {ahead['name']}, one ahead, is ready. Wait until {head['name']} has landed")
        else:
            print(f"build:    nothing; {head['name']}, the first change, is not ready ({head['state']})")
            print(f"describe: {head['name']} ({head['state']})")
    print("problems: " + ("none" if not problems else ""))
    for problem in problems:
        print(f"  - {problem}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
