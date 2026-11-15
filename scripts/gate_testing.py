"""Temporary repositories, bare remotes and a fake gh for the tests of scripts/gate.py. Nothing here touches the
repository the tests run in: every repository, remote, hook and record lives in a temporary directory."""

from __future__ import annotations

import json
import os
import shlex
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

SCRIPTS = Path(__file__).resolve().parent
GATE = SCRIPTS / "gate.py"
REAL_POLICY = json.loads((SCRIPTS / "gate-policy.json").read_text())
PASS = ["python3", "-c", "pass"]


def policy(full=None, docs=None, e2e=None) -> dict:
    """The real policy's allowlist and settings with fast fake commands."""
    value = json.loads(json.dumps(REAL_POLICY))
    value["kinds"] = {
        "full": full or [{"name": "full-ok", "argv": PASS}],
        "docs": docs or [{"name": "docs-ok", "argv": PASS}],
        "e2e": e2e or [{"name": "e2e-ok", "argv": PASS}],
    }
    return value


def flag_command(name: str, flag: Path, action: str = "fail") -> dict:
    """A command that fails, sleeps or writes into its checkout only while the flag file exists."""
    code = {
        "fail": "sys.exit(1)",
        "sleep": "time.sleep(60)",
        "write": "open('stray.txt', 'w').write('x')",
        "orphan": "os.closerange(3, 1024); time.sleep(60)",
    }[action]
    script = f"import os, sys, time\nif os.path.exists({str(flag)!r}):\n    {code}\n"
    return {"name": name, "argv": ["python3", "-c", script]}


def isolated_env(extra: dict | None = None) -> dict:
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.update(
        GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1", GIT_AUTHOR_NAME="Gate test",
        GIT_AUTHOR_EMAIL="gate@example.test", GIT_COMMITTER_NAME="Gate test", GIT_COMMITTER_EMAIL="gate@example.test",
        PYTHONDONTWRITEBYTECODE="1",
    )
    env.update(extra or {})
    return env


class Sandbox:
    """A temporary directory with a repository whose committed gate.py is the real one."""

    def __init__(self, test, policy_value=None, files=None):
        self.temp = tempfile.TemporaryDirectory()
        test.addCleanup(self.cleanup)
        self.dir = Path(self.temp.name).resolve()
        self.cargo_home = self.dir / "cargo-home"
        self.cargo_home.mkdir()
        self.env = isolated_env({"CARGO_HOME": str(self.cargo_home)})
        self.root = self.dir / "repo"
        self.root.mkdir()
        self.git("init", "-q", "-b", "main")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "tag.gpgsign", "false")
        exclude = self.root / ".git" / "info" / "exclude"
        exclude.parent.mkdir(parents=True, exist_ok=True)
        exclude.write_text(".claude/worktrees/\n")
        self.write("scripts/gate.py", GATE.read_text())
        self.write("scripts/gate-policy.json", json.dumps(policy_value or policy(), indent=2) + "\n")
        self.write("README.md", "# Test\n")
        for path, text in (files or {}).items():
            self.write(path, text)
        self.commit("Initial")
        self.common = self.root / ".git"
        self.state = self.common / "cicada-gate"
        self.owned = self.root / ".claude" / "worktrees" / "gate"

    def cleanup(self):
        # Stop anything a test left in its process groups before the directory goes.
        marker = self.state / "in-progress.json" if hasattr(self, "state") else None
        if marker and marker.exists():
            try:
                child = json.loads(marker.read_text()).get("child")
                if child:
                    os.killpg(child["pgid"], 9)
            except (ValueError, OSError, KeyError):
                pass
        self.temp.cleanup()

    def git(self, *args, cwd=None, check=True, input=None) -> str:
        done = subprocess.run(["git", *args], cwd=cwd or self.root, env=self.env, capture_output=True, text=True,
                              input=input)
        if check and done.returncode:
            raise AssertionError(f"git {' '.join(args)} failed: {done.stderr}")
        return done.stdout.strip()

    def write(self, path: str, text: str, mode: int | None = None) -> None:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        if mode is not None:
            target.chmod(mode)

    def commit(self, message: str, files: dict | None = None) -> str:
        for path, text in (files or {}).items():
            self.write(path, text)
        self.git("add", "-A")
        self.git("commit", "-q", "--allow-empty", "-m", message)
        return self.git("rev-parse", "HEAD")

    def gate(self, *args, cwd=None, input=None, env=None) -> subprocess.CompletedProcess:
        return subprocess.run([sys.executable, "-B", str((cwd or self.root) / "scripts" / "gate.py"), *args],
                              cwd=cwd or self.root, env={**self.env, **(env or {})}, input=input, capture_output=True,
                              text=True)

    def spawn_gate(self, *args) -> subprocess.Popen:
        """A gate in the background; its output goes to files, so commands it leaves behind hold no pipe."""
        log = self.dir / "spawned.log"
        with open(log, "w") as output:
            return subprocess.Popen([sys.executable, "-B", str(self.root / "scripts" / "gate.py"), *args],
                                    cwd=self.root, env=self.env, stdout=output, stderr=output)

    def spawned_log(self) -> str:
        return (self.dir / "spawned.log").read_text()

    def record(self, kind: str, rev: str = "HEAD") -> dict | None:
        path = self.state / "records" / f"{kind}-{self.git('rev-parse', rev + '^{tree}')}.json"
        return json.loads(path.read_text()) if path.exists() else None

    def marker(self) -> dict | None:
        path = self.state / "in-progress.json"
        return json.loads(path.read_text()) if path.exists() else None

    def wait_for_child(self, timeout=30.0, argv_contains: str | None = None) -> dict:
        """The marker once it records a running command (whose argv holds argv_contains, if given)."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                value = self.marker()
            except ValueError:
                value = None  # caught mid-write
            child = (value or {}).get("child")
            if child and child.get("start") and (argv_contains is None or argv_contains in " ".join(child["argv"])):
                return value
            time.sleep(0.05)
        raise AssertionError("the gate never started its command")

    def bare(self, name="origin") -> Path:
        path = self.dir / f"{name}.git"
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(path)], env=self.env, check=True)
        self.git("remote", "add", name, str(path))
        return path


def group_gone(pgid: int, timeout=10.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            os.killpg(pgid, 0)
        except ProcessLookupError:
            return True
        except PermissionError:
            pass  # macOS answers EPERM while the group holds only a zombie nobody has reaped yet
        time.sleep(0.05)
    return False


FAKE_GH = r'''#!/usr/bin/env python3
"""A fake gh for the release tests: releases in a JSON file, failures injected by name."""
import hashlib, json, os, subprocess, sys
path = os.environ["FAKE_GH_STATE"]
state = json.load(open(path))
args = sys.argv[1:]
state.setdefault("calls", []).append(args)
releases = state.setdefault("releases", [])

def save():
    json.dump(state, open(path, "w"), indent=2)

def fail(name):
    if name == state.get("sleep"):
        import time
        time.sleep(60)
    if name in state.get("fail", []):
        state["fail"].remove(name)
        save()
        print(f"injected failure: {name}", file=sys.stderr)
        sys.exit(1)

def opt(name):
    return args[args.index(name) + 1]

def asset(file):
    data = open(file, "rb").read()
    state["next_id"] = state.get("next_id", 100) + 1
    blob = os.path.join(os.path.dirname(path), f"asset-{state['next_id']}")
    open(blob, "wb").write(data)
    digest = "sha256:" + hashlib.sha256(data).hexdigest() if state.get("digests", True) else None
    return {"id": state["next_id"], "name": os.path.basename(file), "digest": digest, "file": blob}

def find(tag):
    return [r for r in releases if r["tag_name"] == tag]

if args[:2] == ["api", "--paginate"]:
    if args[2] != f"repos/{state['repo']}/releases":
        sys.exit(f"unexpected repository {args[2]}")
    fail("list")
    save()
    for r in releases:
        print(json.dumps({k: r[k] for k in ("id", "tag_name", "name", "draft", "body")}
                         | {"assets": [{k: a[k] for k in ("id", "name", "digest")} for a in r["assets"]]}))
elif args[:2] == ["release", "create"]:
    if opt("--repo") != state["repo"] or "--draft" not in args or "--verify-tag" not in args:
        sys.exit("unexpected create")
    fail("create")
    tag = args[2]
    remote = subprocess.run(["git", "--git-dir", state["origin"], "rev-parse", "--verify", "-q", f"refs/tags/{tag}"],
                            capture_output=True)
    if remote.returncode:
        sys.exit("tag does not exist remotely (--verify-tag)")
    state["next_id"] = state.get("next_id", 100) + 1
    releases.append({"id": state["next_id"], "tag_name": tag, "name": opt("--title"), "draft": True,
                     "body": open(opt("--notes-file")).read(), "assets": [asset(args[3])]})
    save()
    fail("create-after")
elif args[:2] == ["release", "upload"]:
    if opt("--repo") != state["repo"]:
        sys.exit("unexpected repository")
    fail("upload")
    target = find(args[2])[0]
    if any(a["name"] == os.path.basename(args[3]) for a in target["assets"]):
        sys.exit("asset exists (no --clobber)")
    target["assets"].append(asset(args[3]))
    save()
elif args[:3] == ["api", "-X", "PATCH"]:
    fail("publish")
    rid = int(args[3].rsplit("/", 1)[1])
    for r in releases:
        if r["id"] == rid and args[3] == f"repos/{state['repo']}/releases/{rid}":
            r["draft"] = False
    save()
elif args[0] == "api" and "/releases/assets/" in args[1]:
    aid = int(args[1].rsplit("/", 1)[1])
    for r in releases:
        for a in r["assets"]:
            if a["id"] == aid:
                save()
                sys.stdout.buffer.write(open(a["file"], "rb").read())
                sys.exit(0)
    sys.exit("no such asset")
else:
    sys.exit(f"fake gh does not know {args}")
'''


def install_fake_gh(sandbox: Sandbox, origin: Path, repo: str = "acme/cicada") -> Path:
    bin_dir = sandbox.dir / "bin"
    bin_dir.mkdir(exist_ok=True)
    gh = bin_dir / "gh"
    gh.write_text(FAKE_GH)
    gh.chmod(0o755)
    state = sandbox.dir / "gh" / "state.json"
    state.parent.mkdir(exist_ok=True)
    state.write_text(json.dumps({"repo": repo, "origin": str(origin), "releases": [], "calls": []}))
    sandbox.env["PATH"] = f"{bin_dir}{os.pathsep}{sandbox.env['PATH']}"
    sandbox.env["FAKE_GH_STATE"] = str(state)
    return state


def copy_script(sandbox: Sandbox, name: str, mode: int = 0o755) -> None:
    shutil.copyfile(SCRIPTS / name, sandbox.root / "scripts" / name)
    (sandbox.root / "scripts" / name).chmod(mode)


def install_blocking_rustc(sandbox: Sandbox, flag: Path) -> None:
    """A rustc first on PATH whose `-V` blocks while flag exists, and otherwise answers like rustc."""
    bin_dir = sandbox.dir / "bin"
    bin_dir.mkdir(exist_ok=True)
    rustc = bin_dir / "rustc"
    rustc.write_text(f"#!/bin/sh\nwhile [ -e {shlex.quote(str(flag))} ]; do sleep 0.1; done\necho 'rustc fake'\n")
    rustc.chmod(0o755)
    sandbox.env["PATH"] = f"{bin_dir}{os.pathsep}{sandbox.env['PATH']}"
