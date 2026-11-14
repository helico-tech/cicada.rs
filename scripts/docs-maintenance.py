#!/usr/bin/env python3
"""Local documentation ownership and a durable cursor for the docs session."""

import argparse
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile


def git(*args, check=True):
    return subprocess.run(["git", *args], check=check, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout


def protected(path):
    """Handwritten user docs; decisions and generated artifacts stay with their producing change."""
    return path == "README.md" or (
        path.startswith("docs/")
        and not path.startswith(("docs/adr/", "docs/reference/contracts/"))
        and path not in ("docs/openapi.json", "docs/reference/cli.md")
    )


def atomic_write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as output:
            json.dump(value, output, indent=2)
            output.write("\n")
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def read_owner(common):
    path = common / "cicada-docs" / "owner.json"
    if not path.exists():
        raise SystemExit("No docs owner is installed. Run claim in the docs worktree.")
    return path, json.loads(path.read_text())


def version_only(path):
    """Release builders may refresh a README's immutable release pins, with no prose changes."""
    if path != "README.md":
        return False
    before = git("show", "HEAD:README.md", check=False)
    after = git("show", ":README.md", check=False)
    normalize = lambda value: re.sub(rb'tag = "v[0-9]+\.[0-9]+\.[0-9]+"', b'tag = "vX.Y.Z"', value)
    return bool(before and after) and normalize(before) == normalize(after)


def check_owner(common):
    _, owner = read_owner(common)
    root = Path(os.fsdecode(git("rev-parse", "--show-toplevel")).strip()).resolve()
    if root == Path(owner["worktree"]).resolve():
        return
    changed = [os.fsdecode(p) for p in git("diff", "--cached", "--name-only", "-z", "--no-renames").split(b"\0") if p]
    refused = []
    for path in filter(protected, changed):
        if version_only(path):
            continue
        # Main may integrate exactly the documentation prepared on the owner branch.
        staged = subprocess.run(["git", "rev-parse", "--verify", f":{path}"], capture_output=True)
        prepared = subprocess.run(
            ["git", "rev-parse", "--verify", f"refs/heads/{owner['branch']}:{path}"], capture_output=True
        )
        if staged.returncode == prepared.returncode == 0 and staged.stdout == prepared.stdout:
            continue
        if staged.returncode != 0 and prepared.returncode != 0:
            continue  # A deletion already prepared by the docs branch.
        refused.append(path)
    if refused:
        raise SystemExit(
            "Handwritten documentation belongs to the docs session at " + owner["worktree"]
            + ". Send it the code change; do not edit these docs here:\n  " + "\n  ".join(refused)
        )


def claim(common):
    root = Path(os.fsdecode(git("rev-parse", "--show-toplevel")).strip()).resolve()
    branch = os.fsdecode(git("branch", "--show-current")).strip()
    if branch != "docs":
        raise SystemExit("Claim ownership from branch docs.")
    state = common / "cicada-docs" / "owner.json"
    if state.exists():
        current = json.loads(state.read_text())
        if Path(current["worktree"]).resolve() != root:
            raise SystemExit("Another worktree owns the docs; release it there first.")
    # Do not replace existing hooks or a custom hook manager.
    hook_dir = Path(os.fsdecode(git("rev-parse", "--git-path", "hooks")).strip()).resolve()
    marker = "# cicada-docs-owner\n"
    hooks = [hook_dir / name for name in ("pre-commit", "pre-merge-commit")]
    for hook in hooks:
        if hook.exists() and marker not in hook.read_text():
            raise SystemExit(f"Existing hook left untouched: {hook}. Chain check-owner explicitly before claiming.")
    value = {"worktree": str(root), "branch": branch, "reviewed_main": os.fsdecode(git("merge-base", "HEAD", "main")).strip()}
    if state.exists():
        value.update(json.loads(state.read_text()))
    command = shlex.quote(str(root / "scripts" / "docs-maintenance.py"))
    for hook in hooks:
        hook.parent.mkdir(parents=True, exist_ok=True)
        hook.write_text("#!/bin/sh\n" + marker + f"exec python3 {command} check-owner\n")
        hook.chmod(0o755)
    atomic_write(state, value)
    print(json.dumps(value, indent=2))


def scan(common):
    _, owner = read_owner(common)
    cursor = owner["reviewed_main"]
    result = {"reviewed_main": cursor, "refs": {}}
    for ref in ("main", "origin/main"):
        resolved = subprocess.run(["git", "rev-parse", "--verify", ref], capture_output=True)
        if resolved.returncode:
            continue
        sha = os.fsdecode(resolved.stdout).strip()
        ancestor = subprocess.run(["git", "merge-base", "--is-ancestor", cursor, sha], capture_output=True)
        result["refs"][ref] = {
            "sha": sha,
            "linear_since_cursor": ancestor.returncode == 0,
            "commits": os.fsdecode(git("log", "--oneline", f"{cursor}..{sha}" )).splitlines(),
            "files": [os.fsdecode(p) for p in git("diff", "--name-only", "-z", cursor, sha).split(b"\0") if p],
        }
    print(json.dumps(result, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["claim", "check-owner", "scan", "acknowledge", "status", "release"])
    parser.add_argument("revision", nargs="?")
    args = parser.parse_args()
    common = Path(os.fsdecode(git("rev-parse", "--git-common-dir")).strip()).resolve()
    if args.action == "claim":
        claim(common)
    elif args.action == "check-owner":
        check_owner(common)
    elif args.action == "scan":
        scan(common)
    else:
        path, owner = read_owner(common)
        root = Path(os.fsdecode(git("rev-parse", "--show-toplevel")).strip()).resolve()
        if args.action != "status" and root != Path(owner["worktree"]).resolve():
            raise SystemExit("Only the owner worktree can change the maintenance cursor or release ownership.")
        if args.action == "acknowledge":
            if not args.revision:
                parser.error("acknowledge needs the exact reviewed main commit")
            sha = os.fsdecode(git("rev-parse", "--verify", args.revision + "^{commit}")).strip()
            if subprocess.run(["git", "merge-base", "--is-ancestor", sha, "main"], capture_output=True).returncode:
                raise SystemExit("The reviewed commit must be on main.")
            owner["reviewed_main"] = sha
            atomic_write(path, owner)
        elif args.action == "release":
            hook_dir = Path(os.fsdecode(git("rev-parse", "--git-path", "hooks")).strip()).resolve()
            for name in ("pre-commit", "pre-merge-commit"):
                hook = hook_dir / name
                if hook.exists() and "# cicada-docs-owner\n" in hook.read_text():
                    hook.unlink()
            path.unlink()
        print(json.dumps(owner, indent=2))


if __name__ == "__main__":
    main()
