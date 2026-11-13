#!/usr/bin/env python3
"""Prints the open OpenSpec changes in their order, the state of each, and what the design and build loops do next
(ADR 0030).

The order comes from the `## Order` section of each proposal ("After `x`", "before `y`"). The state comes from the
files of the change: only a proposal, described (design or spec), ready (a `tasks.md`), in build (some tasks ticked),
built (all ticked). While a change is being built, its `tasks.md` is read from its worktree. The design loop describes
the first change without a `tasks.md`, unless two ready changes already wait for the build loop.

    status.py          print the order and what to do
    status.py --wait   wait until the first change in the order is ready, then print
"""

import pathlib
import re
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
WAITING = 2
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


def order():
    """The changes by name, the queue of their names in order, and the problems with the order."""
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
    return names, changes, queue, problems


def main():
    if sys.argv[1:] == ["--wait"]:
        while True:
            _, changes, queue, _ = order()
            if queue and changes[queue[0]]["tasks"] != "-":
                break
            time.sleep(30)
    elif sys.argv[1:]:
        print(__doc__)
        return 2
    names, changes, queue, problems = order()

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
        else:
            print(f"build:    nothing; {head['name']}, the first change, is not ready ({head['state']})")
        waiting = [n for n in queue if changes[n]["state"] == "ready" and changes[n]["tree"] == "-"]
        first_open = next((n for n in queue if changes[n]["tasks"] == "-"), None)
        if first_open is None:
            print("describe: nothing; every open change is ready")
        elif len(waiting) >= WAITING:
            print(f"describe: nothing; {' and '.join(waiting)} wait for the build loop")
        else:
            print(f"describe: {first_open} ({changes[first_open]['state']})")
    print("problems: " + ("none" if not problems else ""))
    for problem in problems:
        print(f"  - {problem}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
