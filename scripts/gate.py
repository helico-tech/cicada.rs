#!/usr/bin/env python3
"""Local check evidence, the pre-push gate and the pinned release driver (ADR 0046).

    gate.py checks --scope full|docs|e2e   run a policy in the current directory and record nothing (manual CI)
    gate.py run [REV] [--e2e] [--force]    check REV's exact tree in the owned worktree and record the evidence
    gate.py status [REV]                   say what evidence REV has and what a push of it needs
    gate.py recover                        clear an abandoned run once its processes are gone
    gate.py install-hook                   install the pre-push gate (the owner, once per repository)
    gate.py pre-push REMOTE URL            the hook: refuse main and version-tag updates without evidence
    gate.py release vX.Y.Z [REV]           publish a checked release (only on the owner's yes, in the build loop)

The policy is scripts/gate-policy.json, read from the checked commit's Git objects, and must be run by that commit's
own gate.py. Records live in Git's common directory under cicada-gate/, so every worktree shares them. This is local
workflow control: --no-verify, another clone, an old hook or a forged record bypasses it, and the checks run in a
controlled environment, not a hermetic build. Python 3.11 or newer, standard library only.
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import io
import json
import os
import platform
import re
import shlex
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlsplit

SCHEMA = 1
POLICY_PATH = "scripts/gate-policy.json"
RUNNER_PATH = "scripts/gate.py"
STATE_DIR = "cicada-gate"
OWNED = ".claude/worktrees/gate"
OWNER_FILE = "cicada-gate-owner"
HOOK_MARKER = "# cicada-gate-pre-push"
SEARCH_LIMIT = 100
KINDS = ("full", "docs", "e2e")
VERSION_TAG = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+")
DEFAULT_PG_URL = "postgres://cicada:cicada@localhost:55432/cicada"
# The caller's environment that reaches check commands; everything else (update flags, test filters, compiler flags
# and wrappers, target directories) is dropped.
KEEP_ENV = {
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LANGUAGE", "TERM", "CARGO_HOME", "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN", "DOCKER_HOST", "DOCKER_CONFIG", "DOCKER_CONTEXT", "DOCKER_CERT_PATH", "DOCKER_TLS_VERIFY",
    "SSH_AUTH_SOCK",
}
# Set by Git for hooks; dropped so that a git call with an explicit directory finds that directory's repository.
GIT_LOCATION = {"GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_PREFIX", "GIT_OBJECT_DIRECTORY"}


class GateError(Exception):
    """A refusal or failure after which nothing is left running."""


class Uncertain(Exception):
    """Work may still be running; the in-progress marker stays until `recover`."""


class Interrupted(BaseException):
    """INT or TERM reached the gate."""


def now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def is_zero(oid: str) -> bool:
    return bool(oid) and set(oid) == {"0"}


# --- Git ----------------------------------------------------------------------------------------------------------


def git_env() -> dict[str, str]:
    return {k: v for k, v in os.environ.items() if k not in GIT_LOCATION}


def git(args, cwd, check=True, env=None, input=None) -> subprocess.CompletedProcess:
    done = subprocess.run(["git", *args], cwd=cwd, capture_output=True, input=input, env=env or git_env())
    if check and done.returncode:
        raise GateError(f"git {' '.join(args)}: {done.stderr.decode(errors='replace').strip()}")
    return done


def out(args, cwd, **kw) -> str:
    return git(args, cwd, **kw).stdout.decode().strip()


class Repo:
    def __init__(self, cwd):
        self.cwd = Path(cwd).resolve()
        self.common = Path(out(["rev-parse", "--path-format=absolute", "--git-common-dir"], self.cwd)).resolve()
        self.state = self.common / STATE_DIR

    def git(self, *args, **kw):
        return git(list(args), self.cwd, **kw)

    def out(self, *args, **kw) -> str:
        return out(list(args), self.cwd, **kw)

    def commit(self, rev: str) -> str:
        done = self.git("rev-parse", "--verify", "--quiet", "--end-of-options", f"{rev}^{{commit}}", check=False)
        if done.returncode:
            raise GateError(f"{rev} is not a commit")
        return done.stdout.decode().strip()

    def tree(self, commit: str) -> str:
        return self.out("rev-parse", f"{commit}^{{tree}}")

    def entry(self, commit: str, path: str) -> tuple[str, str, str] | None:
        """Mode, type and object of a path in a commit's tree."""
        line = self.out("ls-tree", "-z", commit, "--", path).rstrip("\0")
        if not line:
            return None
        meta, _ = line.split("\t", 1)
        mode, kind, oid = meta.split()
        return mode, kind, oid

    def blob(self, commit: str, path: str) -> str | None:
        found = self.entry(commit, path)
        return found[2] if found and found[1] == "blob" else None

    def show(self, oid: str) -> bytes:
        return self.git("cat-file", "blob", oid).stdout

    def exists(self, oid: str) -> bool:
        return self.git("cat-file", "-e", oid, check=False).returncode == 0

    def is_ancestor(self, older: str, newer: str) -> bool:
        return self.git("merge-base", "--is-ancestor", older, newer, check=False).returncode == 0

    def ref(self, name: str) -> str | None:
        done = self.git("rev-parse", "--verify", "--quiet", name, check=False)
        return done.stdout.decode().strip() if done.returncode == 0 else None

    def first_parents(self, commit: str) -> list[str]:
        """Up to SEARCH_LIMIT first-parent ancestors, nearest first."""
        listed = self.out("rev-list", "--first-parent", f"--max-count={SEARCH_LIMIT + 1}", commit).split()
        return listed[1:]

    def main_worktree(self) -> Path:
        lines = self.out("worktree", "list", "--porcelain").splitlines()
        if not lines or not lines[0].startswith("worktree ") or (len(lines) > 1 and lines[1] == "bare"):
            raise GateError("the repository has no main worktree")
        return Path(lines[0][len("worktree "):]).resolve()

    def worktrees(self) -> set[Path]:
        return {
            Path(line[len("worktree "):]).resolve()
            for line in self.out("worktree", "list", "--porcelain").splitlines()
            if line.startswith("worktree ")
        }

    def hash_file(self, path: Path, as_path: str) -> str:
        return self.out("hash-object", f"--path={as_path}", "--", str(path))


# --- Policy and environment ---------------------------------------------------------------------------------------


def parse_policy(data: bytes) -> dict:
    """The policy, or GateError when its schema is unsupported or a field is malformed."""
    try:
        policy = json.loads(data)
    except ValueError as error:
        raise GateError(f"the policy is not JSON: {error}") from None
    if not isinstance(policy, dict) or policy.get("schema") != SCHEMA:
        raise GateError(f"unsupported policy schema {policy.get('schema') if isinstance(policy, dict) else None!r}")

    def strings(value, what):
        if not isinstance(value, dict) or not all(isinstance(k, str) and isinstance(v, str) for k, v in value.items()):
            raise GateError(f"policy: {what} must map names to strings")

    strings(policy.get("env", {}), "env")
    if not isinstance(policy.get("pg_url_variable"), str) or not policy["pg_url_variable"]:
        raise GateError("policy: pg_url_variable is missing")
    kinds = policy.get("kinds")
    if not isinstance(kinds, dict) or set(kinds) != set(KINDS):
        raise GateError(f"policy: kinds must be exactly {', '.join(KINDS)}")
    for kind, commands in kinds.items():
        if not isinstance(commands, list) or not commands:
            raise GateError(f"policy: {kind} has no commands")
        names = set()
        for command in commands:
            argv = command.get("argv") if isinstance(command, dict) else None
            name = command.get("name") if isinstance(command, dict) else None
            if not isinstance(name, str) or not name or name in names:
                raise GateError(f"policy: {kind} has a command without a unique name")
            if not isinstance(argv, list) or not argv or not all(isinstance(a, str) and a for a in argv):
                raise GateError(f"policy: {kind}/{name} needs an argv list of strings")
            strings(command.get("env", {}), f"{kind}/{name} env")
            names.add(name)
    allow = policy.get("docs_allowlist")
    if not isinstance(allow, dict) or set(allow) != {"files", "markdown_under", "except_under"}:
        raise GateError("policy: docs_allowlist needs files, markdown_under and except_under")
    for key, value in allow.items():
        if not isinstance(value, list) or not all(isinstance(v, str) and v for v in value):
            raise GateError(f"policy: docs_allowlist.{key} must be a list of paths")
    return policy


def load_policy(repo: Repo, commit: str) -> tuple[str, dict]:
    blob = repo.blob(commit, POLICY_PATH)
    if blob is None:
        raise GateError(f"{commit} has no {POLICY_PATH}")
    return blob, parse_policy(repo.show(blob))


def expected_commands(policy: dict, kind: str) -> list[dict]:
    return [{"name": c["name"], "argv": c["argv"], "env": c.get("env", {})} for c in policy["kinds"][kind]]


def pg_url() -> str:
    url = os.environ.get("CICADA_GATE_PG_URL", DEFAULT_PG_URL)
    if not url.strip():
        raise GateError("CICADA_GATE_PG_URL is empty: the full checks need PostgreSQL")
    return url


def pg_metadata(url: str) -> dict:
    """Host, port and database only: no user, password or query fields."""
    try:
        parts = urlsplit(url)
        return {"host": parts.hostname, "port": parts.port, "database": parts.path.lstrip("/") or None}
    except ValueError:
        return {"host": None, "port": None, "database": None}


def pg_diagnostic(url: str) -> None:
    """A hint when nothing answers; it proves nothing, the tests themselves must connect."""
    meta = pg_metadata(url)
    if not meta["host"]:
        return
    try:
        socket.create_connection((meta["host"], meta["port"] or 5432), timeout=2).close()
    except OSError:
        print(f"gate: PostgreSQL at {meta['host']}:{meta['port']} does not answer; its tests will fail", file=sys.stderr)


def controlled_env() -> dict[str, str]:
    return {k: v for k, v in os.environ.items() if k in KEEP_ENV or k.startswith("LC_")}


def command_env(policy: dict, url: str, extra: dict | None = None) -> dict[str, str]:
    env = controlled_env()
    env.update(policy.get("env", {}))
    env[policy["pg_url_variable"]] = url
    env.update(extra or {})
    return env


# --- Records ------------------------------------------------------------------------------------------------------


def atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(name, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def write_json(path: Path, value) -> None:
    atomic_write(path, (json.dumps(value, indent=2, sort_keys=True) + "\n").encode())


def record_path(repo: Repo, kind: str, tree: str) -> Path:
    return repo.state / "records" / f"{kind}-{tree}.json"


@dataclass
class Check:
    """What one record says for a commit: missing, running, failed, invalid or pass."""

    status: str
    reason: str = ""
    record: dict | None = None


def check_record(repo: Repo, kind: str, commit: str, policy_blob: str, policy: dict, runner_blob: str) -> Check:
    tree = repo.tree(commit)
    path = record_path(repo, kind, tree)
    if not path.exists():
        return Check("missing", f"no {kind} record for tree {tree}")
    try:
        record = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        return Check("invalid", f"{path.name} is unreadable: {error}")
    if not isinstance(record, dict) or record.get("schema") != SCHEMA:
        return Check("invalid", f"{path.name} has an unknown schema")
    if record.get("kind") != kind or record.get("tree") != tree:
        return Check("invalid", f"{path.name} names another kind or tree")
    status = record.get("status")
    if status in ("running", "failed"):
        return Check(status, f"the last {kind} run of tree {tree} is {status}", record)
    if status != "pass":
        return Check("invalid", f"{path.name} has status {status!r}")
    checked = record.get("commit")
    if not isinstance(checked, str) or not repo.exists(checked) or repo.tree(checked) != tree:
        return Check("invalid", f"{path.name} names a commit that is missing or has another tree")
    if record.get("policy_blob") != policy_blob or record.get("runner_blob") != runner_blob:
        return Check("invalid", f"{path.name} was made by another policy or runner")
    commands = record.get("commands")
    want = expected_commands(policy, kind)
    if not isinstance(commands, list) or len(commands) != len(want):
        return Check("invalid", f"{path.name} does not list exactly the policy's {kind} commands")
    for found, wanted in zip(commands, want):
        if not isinstance(found, dict) or {k: found.get(k) for k in wanted} != wanted or found.get("exit_code") != 0:
            return Check("invalid", f"{path.name}: command {wanted['name']} differs or did not pass")
    if not record.get("finished_at"):
        return Check("invalid", f"{path.name} has no finish time")
    return Check("pass", "", record)


# --- Documentation reuse ------------------------------------------------------------------------------------------


def allowed_path(path: str, allow: dict) -> bool:
    if path in allow["files"]:
        return True
    if not path.endswith(".md") or any(path.startswith(p) for p in allow["except_under"]):
        return False
    return any(path.startswith(p) for p in allow["markdown_under"])


def classify(repo: Repo, base: str, target: str, allow: dict) -> list[str]:
    """The changes from base to target that rule out the docs shortcut; empty when every one qualifies."""
    raw = repo.git(
        "diff", "--raw", "-z", "--no-renames", "--no-ext-diff", "--no-textconv", "--ignore-submodules=none",
        base, target,
    ).stdout
    fields = raw.split(b"\0")
    refused = []
    i = 0
    while i < len(fields) - 1:
        header = fields[i].decode()
        path = os.fsdecode(fields[i + 1])
        i += 2
        old_mode, new_mode, _, _, status = header.lstrip(":").split()
        regular = {
            "A": new_mode == "100644", "M": old_mode == new_mode == "100644", "D": old_mode == "100644",
        }.get(status[0], False)
        if not (regular and allowed_path(path, allow)):
            refused.append(f"{status} {old_mode}->{new_mode} {path}")
    return refused


def docs_base(repo: Repo, commit: str, policy_blob: str, policy: dict, runner_blob: str) -> tuple[str | None, str]:
    """The nearest first-parent ancestor with a direct full pass, if only allowed Markdown changed since."""
    for base in repo.first_parents(commit):
        try:
            base_blob, base_policy = load_policy(repo, base)
        except GateError:
            continue
        base_runner = repo.blob(base, RUNNER_PATH)
        if (base_blob, base_runner) != (policy_blob, runner_blob):
            continue
        if check_record(repo, "full", base, base_blob, base_policy, base_runner).status != "pass":
            continue
        refused = classify(repo, base, commit, policy["docs_allowlist"])
        if refused:
            return None, f"since the full pass of {base[:12]}: " + "; ".join(refused[:5])
        return base, ""
    return None, f"no direct full pass among {SEARCH_LIMIT} first-parent ancestors"


@dataclass
class Auth:
    ok: bool
    how: str


def identity(repo: Repo, commit: str) -> tuple[str, dict, str]:
    policy_blob, policy = load_policy(repo, commit)
    runner = repo.blob(commit, RUNNER_PATH)
    if runner is None:
        raise GateError(f"{commit} has no {RUNNER_PATH}")
    return policy_blob, policy, runner


def checks_auth(repo: Repo, commit: str) -> Auth:
    try:
        policy_blob, policy, runner = identity(repo, commit)
    except GateError as error:
        return Auth(False, str(error))
    full = check_record(repo, "full", commit, policy_blob, policy, runner)
    if full.status == "pass":
        return Auth(True, f"full pass of {full.record['commit'][:12]}")
    if full.status != "missing":
        return Auth(False, f"full: {full.reason}")
    docs = check_record(repo, "docs", commit, policy_blob, policy, runner)
    if docs.status != "pass":
        return Auth(False, f"docs: {docs.reason}" if docs.status != "missing" else "no checks evidence")
    base = (docs.record.get("base") or {}).get("commit")
    if not isinstance(base, str) or base not in repo.first_parents(commit):
        return Auth(False, "docs: its base is not a first-parent ancestor within the search window")
    if repo.tree(base) != docs.record["base"].get("tree"):
        return Auth(False, "docs: its base tree differs")
    if check_record(repo, "full", base, policy_blob, policy, runner).status != "pass":
        return Auth(False, f"docs: base {base[:12]} has no valid direct full pass")
    refused = classify(repo, base, commit, policy["docs_allowlist"])
    if refused:
        return Auth(False, "docs: " + "; ".join(refused[:5]))
    return Auth(True, f"docs pass over the full pass of {base[:12]}")


def e2e_auth(repo: Repo, commit: str) -> Auth:
    try:
        policy_blob, policy, runner = identity(repo, commit)
    except GateError as error:
        return Auth(False, str(error))
    direct = check_record(repo, "e2e", commit, policy_blob, policy, runner)
    if direct.status == "pass":
        return Auth(True, f"e2e pass of {direct.record['commit'][:12]}")
    if direct.status != "missing":
        return Auth(False, f"e2e: {direct.reason}")
    base, why = docs_base(repo, commit, policy_blob, policy, runner)
    if base and check_record(repo, "e2e", base, policy_blob, policy, runner).status == "pass":
        return Auth(True, f"e2e pass of the full-checked base {base[:12]}")
    return Auth(False, "no e2e evidence" + (f" ({why})" if why else ""))


# --- Lock, marker and processes ------------------------------------------------------------------------------------


def acquire_lock(repo: Repo) -> int:
    """The common lock; child commands inherit it, so a surviving child keeps it held."""
    repo.state.mkdir(parents=True, exist_ok=True)
    fd = os.open(repo.state / "lock", os.O_RDWR | os.O_CREAT, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        os.close(fd)
        raise GateError("busy: another gate command, or a command it started, holds the lock") from None
    except OSError as error:
        os.close(fd)
        raise GateError(f"cannot take the gate lock here: {error}") from None
    os.set_inheritable(fd, True)
    return fd


def start_time(pid: int) -> str | None:
    done = subprocess.run(["ps", "-o", "lstart=", "-p", str(pid)], capture_output=True, text=True)
    return done.stdout.strip() or None


def processes() -> dict[int, tuple[int, str]]:
    """pid -> (process group, start time) of every visible process."""
    done = subprocess.run(["ps", "-A", "-o", "pid=", "-o", "pgid=", "-o", "lstart="], capture_output=True, text=True)
    if done.returncode:
        raise Uncertain("ps cannot list processes")
    found = {}
    for line in done.stdout.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3:
            found[int(parts[0])] = (int(parts[1]), parts[2].strip())
    return found


def group_alive(pgid: int) -> bool:
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def stop_group(pgid: int, proc: subprocess.Popen | None = None) -> bool:
    """INT, then TERM, then KILL the group; True once it has no members left."""
    for sig, wait in ((signal.SIGINT, 60), (signal.SIGTERM, 15), (signal.SIGKILL, 10)):
        try:
            os.killpg(pgid, sig)
        except ProcessLookupError:
            pass
        deadline = time.monotonic() + wait
        while time.monotonic() < deadline:
            if proc is not None:
                proc.poll()
            if not group_alive(pgid):
                return True
            time.sleep(0.1)
    return not group_alive(pgid)


class Marker:
    """cicada-gate/in-progress.json: written before a session's work, removed only after a normal completion."""

    def __init__(self, repo: Repo):
        self.path = repo.state / "in-progress.json"
        self.value: dict = {}

    def read(self) -> dict | None:
        try:
            return json.loads(self.path.read_text())
        except FileNotFoundError:
            return None
        except ValueError:
            return {"unreadable": True}

    def begin(self, what: str, worktree: Path) -> None:
        if self.path.exists():
            raise GateError(f"an earlier gate run was abandoned ({self.path}); run `gate.py recover` first")
        pid = os.getpid()
        self.value = {"pid": pid, "start": start_time(pid), "what": what, "worktree": str(worktree),
                      "since": now(), "child": None}
        write_json(self.path, self.value)

    def child(self, pgid: int | None, argv: list[str] | None = None) -> None:
        self.value["child"] = None if pgid is None else {"pgid": pgid, "start": start_time(pgid), "argv": argv}
        write_json(self.path, self.value)

    def clear(self) -> None:
        self.path.unlink(missing_ok=True)


@dataclass
class Session:
    repo: Repo
    lock: int
    marker: Marker
    worktree: Path


@contextmanager
def session(repo: Repo, what: str):
    """The lock and the in-progress marker. A refusal or a failed command after a normal exit clears the marker;
    an interruption or an unexpected error leaves it for `recover`."""
    lock = acquire_lock(repo)
    marker = Marker(repo)
    worktree = repo.main_worktree() / OWNED
    marker.begin(what, worktree)
    try:
        yield Session(repo, lock, marker, worktree)
    except GateError:
        marker.clear()
        raise
    else:
        marker.clear()


def execute(sess: Session, argv: list[str], cwd: Path, env: dict) -> int:
    """Run one command in a process group of its own that holds the inherited lock."""
    try:
        proc = subprocess.Popen(argv, cwd=cwd, env=env, pass_fds=(sess.lock,), start_new_session=True)
    except OSError as error:
        print(f"gate: cannot start {argv[0]}: {error}", file=sys.stderr)
        return 127
    sess.marker.child(proc.pid, argv)
    try:
        code = proc.wait()
    except Interrupted:
        stop_group(proc.pid, proc)
        raise
    if group_alive(proc.pid):
        time.sleep(0.5)  # members that exit with their leader
    if group_alive(proc.pid):
        if not stop_group(proc.pid):
            raise Uncertain(f"{argv[0]} left processes in group {proc.pid} that do not stop")
        print(f"gate: {shlex.join(argv)} left processes behind; they were stopped and the command fails", file=sys.stderr)
        code = code or 1
    sess.marker.child(None)
    return code


def install_signals() -> None:
    state = {"raised": False}

    def handler(signum, frame):
        if not state["raised"]:
            state["raised"] = True
            raise Interrupted(signal.Signals(signum).name)

    signal.signal(signal.SIGINT, handler)
    signal.signal(signal.SIGTERM, handler)


# --- The owned worktree ---------------------------------------------------------------------------------------------


def owner_value(path: Path, common: Path) -> dict:
    return {"path": str(path), "common": str(common)}


def verify_owned(repo: Repo, path: Path) -> None:
    if path.is_symlink() or not path.is_dir():
        raise GateError(f"{path} is not a directory the gate owns; it is left untouched")
    if path not in repo.worktrees():
        raise GateError(f"{path} is not a registered worktree; it is left untouched")
    top = Path(out(["rev-parse", "--show-toplevel"], path)).resolve()
    common = Path(out(["rev-parse", "--path-format=absolute", "--git-common-dir"], path)).resolve()
    admin = Path(out(["rev-parse", "--absolute-git-dir"], path))
    try:
        owner = json.loads((admin / OWNER_FILE).read_text())
    except (OSError, ValueError):
        owner = None
    if top != path or common != repo.common or owner != owner_value(path, repo.common):
        raise GateError(f"{path} is not the gate's own worktree of this repository; it is left untouched")


def dirty(path: Path) -> list[str]:
    """Tracked changes and untracked or ignored files outside the owned top-level target/."""
    raw = git(["status", "--porcelain=v1", "-z", "--ignored", "--untracked-files=normal"], path).stdout
    entries, fields, i = [], raw.split(b"\0"), 0
    while i < len(fields):
        entry = os.fsdecode(fields[i])
        i += 1
        if not entry:
            continue
        if entry[0] in "RC":
            i += 1  # the rename's source
        name = entry[3:]
        if name.rstrip("/") != "target" and not name.startswith("target/"):
            entries.append(entry)
    return entries


def verify_source(path: Path, commit: str, tree: str) -> None:
    head = out(["rev-parse", "HEAD"], path)
    if head != commit or out(["rev-parse", "HEAD^{tree}"], path) != tree:
        raise GateError(f"the owned worktree is at {head}, not {commit}")
    if (path / "target").is_symlink():
        raise GateError("the owned worktree's target/ is a symlink")
    changed = dirty(path)
    if changed:
        raise GateError("the owned worktree differs from the commit: " + "; ".join(changed[:10]))


def prepare_owned(sess: Session, commit: str) -> Path:
    """The owned worktree, detached at commit and clean except its target/ cache."""
    repo, path = sess.repo, sess.worktree
    main = repo.main_worktree()
    if git(["check-ignore", "-q", "--", OWNED], main, check=False).returncode:
        raise GateError(f"{OWNED} must be ignored by Git (for example in .git/info/exclude)")
    if repo.entry(commit, "target") is not None:
        raise GateError(f"{commit} tracks target/, the gate's cache")
    if not os.path.lexists(path):
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.parent.resolve() != main / Path(OWNED).parent:
            raise GateError(f"{path.parent} resolves elsewhere; the gate does not follow symlinks")
        git(["worktree", "add", "--detach", "--quiet", str(path), commit], main)
        admin = Path(out(["rev-parse", "--absolute-git-dir"], path))
        write_json(admin / OWNER_FILE, owner_value(path, repo.common))
    verify_owned(repo, path)
    if (path / "target").is_symlink():
        raise GateError(f"{path}/target is a symlink; the gate keeps only a real cache directory")
    git(["checkout", "--quiet", "--detach", "--force", commit], path)
    git(["clean", "-ffdxq", "-e", "/target"], path)
    verify_source(path, commit, repo.tree(commit))
    return path


def refuse_external_config(path: Path, env: dict) -> None:
    for parent in path.parents:
        for name in (".cargo/config", ".cargo/config.toml", "rust-toolchain", "rust-toolchain.toml"):
            if (parent / name).exists():
                raise GateError(f"{parent / name} would change the checks from outside the commit; move it away")
    home = Path(env.get("CARGO_HOME") or os.path.expanduser("~/.cargo"))
    for name in ("config", "config.toml"):
        if (home / name).exists():
            raise GateError(f"{home / name} would change the checks from outside the commit; move it away")


def tool_versions(path: Path, env: dict, kind: str) -> dict:
    def version(*argv):
        try:
            done = subprocess.run(argv, cwd=path, env=env, capture_output=True, text=True, timeout=60)
        except (OSError, subprocess.TimeoutExpired):
            return None
        return done.stdout.strip() if done.returncode == 0 else None

    tools = {"rustc": version("rustc", "-V"), "cargo": version("cargo", "-V"), "git": version("git", "--version"),
             "python": platform.python_version(), "platform": platform.platform()}
    if kind == "e2e":
        tools["docker"] = version("docker", "version", "--format", "{{.Server.Version}}")
    return tools


# --- Running checks ---------------------------------------------------------------------------------------------------


def ensure_runner(repo: Repo, commit: str) -> str:
    blob = repo.blob(commit, RUNNER_PATH)
    if blob is None or repo.hash_file(Path(__file__).resolve(), RUNNER_PATH) != blob:
        raise GateError(f"run the gate of {commit[:12]} itself: this {RUNNER_PATH} differs from that commit's")
    return blob


def execute_kind(sess: Session, commit: str, kind: str, base: str | None) -> None:
    repo = sess.repo
    policy_blob, policy, runner = identity(repo, commit)
    tree = repo.tree(commit)
    url = pg_url()
    path = prepare_owned(sess, commit)
    env = command_env(policy, url)
    refuse_external_config(path, env)
    record = {
        "schema": SCHEMA, "kind": kind, "status": "running", "commit": commit, "tree": tree,
        "policy_blob": policy_blob, "runner_blob": runner, "pg_url_variable": policy["pg_url_variable"],
        "pg": pg_metadata(url), "env": policy.get("env", {}), "started_at": now(), "commands": [],
        "tools": tool_versions(path, env, kind),
    }
    if base:
        record["base"] = {"commit": base, "tree": repo.tree(base)}
    target = record_path(repo, kind, tree)
    write_json(target, record)  # a rerun invalidates the old record before anything runs
    if kind == "full":
        pg_diagnostic(url)
    print(f"gate: {kind} checks of {commit[:12]} in {path}", file=sys.stderr)
    for command in expected_commands(policy, kind):
        entry = dict(command, started_at=now())
        print(f"gate: $ {shlex.join(command['argv'])}", file=sys.stderr)
        code = execute(sess, command["argv"], path, {**env, **command["env"]})
        entry.update(exit_code=code, finished_at=now())
        record["commands"].append(entry)
        if code:
            record.update(status="failed", finished_at=now())
            write_json(target, record)
            raise GateError(f"{kind}: {command['name']} failed with exit code {code}")
        write_json(target, record)
    try:
        verify_source(path, commit, tree)
    except GateError as error:
        record.update(status="failed", finished_at=now(), source_changed=str(error))
        write_json(target, record)
        raise
    record.update(status="pass", finished_at=now())
    write_json(target, record)
    print(f"gate: {kind} passed for tree {tree}", file=sys.stderr)


def ensure_checks(sess: Session, commit: str, force: bool) -> None:
    repo = sess.repo
    policy_blob, policy, runner = identity(repo, commit)
    if not force:
        auth = checks_auth(repo, commit)
        if auth.ok:
            print(f"gate: checks of {commit[:12]}: {auth.how}", file=sys.stderr)
            return
    full = check_record(repo, "full", commit, policy_blob, policy, runner)
    base = None
    if full.status == "missing":
        base, why = docs_base(repo, commit, policy_blob, policy, runner)
        if not base:
            print(f"gate: full checks needed: {why}", file=sys.stderr)
    execute_kind(sess, commit, "docs" if base else "full", base)


def ensure_e2e(sess: Session, commit: str, force: bool) -> None:
    if not force:
        auth = e2e_auth(sess.repo, commit)
        if auth.ok:
            print(f"gate: e2e of {commit[:12]}: {auth.how}", file=sys.stderr)
            return
    execute_kind(sess, commit, "e2e", None)


def run_checks(policy: dict, scope: str, cwd: Path) -> int:
    env = command_env(policy, pg_url())
    for command in expected_commands(policy, scope):
        print(f"gate: $ {shlex.join(command['argv'])}", file=sys.stderr, flush=True)
        try:
            code = subprocess.run(command["argv"], cwd=cwd, env={**env, **command["env"]}).returncode
        except OSError as error:
            print(f"gate: cannot start {command['argv'][0]}: {error}", file=sys.stderr)
            code = 127
        if code:
            print(f"gate: {scope}: {command['name']} failed with exit code {code}", file=sys.stderr)
            return code
    return 0


# --- Status and recovery ------------------------------------------------------------------------------------------


def status_lines(repo: Repo, commit: str) -> list[str]:
    lines = [f"commit {commit} (tree {repo.tree(commit)})"]
    try:
        policy_blob, policy, runner = identity(repo, commit)
    except GateError as error:
        return lines + [f"no evidence possible: {error}"]
    for kind in KINDS:
        found = check_record(repo, kind, commit, policy_blob, policy, runner)
        lines.append(f"{kind} record: {found.status}" + (f" ({found.reason})" if found.reason else ""))
    checks, e2e = checks_auth(repo, commit), e2e_auth(repo, commit)
    lines.append(f"checks: {'yes' if checks.ok else 'no'}, {checks.how}")
    lines.append(f"e2e: {'yes' if e2e.ok else 'no'}, {e2e.how}")
    marker = Marker(repo).read()
    if marker:
        lines.append(f"abandoned or running gate work: {json.dumps(marker)}; `gate.py recover` once it has stopped")
    return lines


def cwd_users(path: Path) -> list[int]:
    """Processes whose working directory is inside path."""
    try:
        done = subprocess.run(["lsof", "-n", "-w", "-d", "cwd", "-F", "pn"], capture_output=True, text=True)
    except OSError:
        raise Uncertain("lsof is missing: cannot tell whether processes still use the owned worktree") from None
    if not done.stdout:
        raise Uncertain("lsof listed no processes: cannot tell whether the owned worktree is in use")
    users, pid = [], None
    for line in done.stdout.splitlines():
        if line.startswith("p"):
            pid = int(line[1:])
        elif line.startswith("n") and pid is not None:
            name = Path(line[1:])
            if name == path or path in name.parents:
                users.append(pid)
    return users


def recover(repo: Repo) -> str:
    lock = acquire_lock(repo)
    try:
        marker = Marker(repo)
        value = marker.read()
        if value is None:
            return "nothing to recover"
        if value.get("unreadable"):
            raise Uncertain(f"{marker.path} is unreadable; inspect it by hand")
        found = processes()
        pid = value.get("pid")
        if pid in found and found[pid][1] == value.get("start"):
            raise Uncertain(f"the gate process {pid} still runs")
        child = value.get("child")
        if child:
            pgid = child["pgid"]
            members = [p for p, (g, _) in found.items() if g == pgid]
            if members:
                leader = found.get(pgid)
                if leader is not None and leader[1] != child.get("start"):
                    raise Uncertain(f"process group {pgid} cannot be identified as the gate's command")
                if not stop_group(pgid):
                    raise Uncertain(f"process group {pgid} does not stop")
        worktree = Path(value.get("worktree", ""))
        users = cwd_users(worktree) if worktree.is_absolute() else []
        if users:
            raise Uncertain(f"processes {users} still work in {worktree}")
        marker.clear()
        return "marker cleared; the interrupted record stays unusable until a new run passes"
    finally:
        os.close(lock)


# --- The pre-push hook --------------------------------------------------------------------------------------------


def hook_text() -> str:
    script = shlex.quote(str(Path(__file__).resolve()))
    return f'#!/bin/sh\n{HOOK_MARKER}\nexec python3 {script} pre-push "$@"\n'


def install_hook(repo: Repo) -> Path:
    hooks = Path(repo.out("rev-parse", "--path-format=absolute", "--git-path", "hooks"))
    hook = hooks / "pre-push"
    if os.path.lexists(hook):
        try:
            current = hook.read_text()
        except (OSError, UnicodeDecodeError):
            current = ""
        if hook.is_symlink() or HOOK_MARKER not in current:
            raise GateError(
                f"{hook} belongs to something else and is left untouched. Chain the gate from it: run\n"
                f"  python3 {shlex.quote(str(Path(__file__).resolve()))} pre-push \"$1\" \"$2\"\n"
                "with the hook's standard input, and fail the push when it fails."
            )
    hooks.mkdir(parents=True, exist_ok=True)
    atomic_write(hook, hook_text().encode())
    hook.chmod(0o755)
    return hook


def pre_push(repo: Repo, remote: str, lines: list[str]) -> list[str]:
    """The refusals for a push, after reading every update."""
    updates, errors = [], []
    for line in lines:
        if not line.strip():
            continue
        parts = line.split()
        if len(parts) != 4:
            errors.append(f"cannot read the update {line!r}")
            continue
        updates.append(parts)
    for _, local, remote_ref, remote_oid in updates:
        if remote_ref == "refs/heads/main":
            errors += main_update(repo, local, remote_oid)
        elif remote_ref.startswith("refs/tags/") and VERSION_TAG.fullmatch(remote_ref[len("refs/tags/"):]):
            errors += tag_update(repo, remote, remote_ref[len("refs/tags/"):], local, remote_oid)
    return errors


def main_update(repo: Repo, local: str, old: str) -> list[str]:
    if is_zero(local):
        return ["deleting main is refused"]
    if not is_zero(old):
        if not repo.exists(old):
            return [f"the remote main {old[:12]} is unknown here: fetch first"]
        if not repo.is_ancestor(old, local):
            return [f"main {local[:12]} does not fast-forward the remote main {old[:12]}"]
    if repo.out("cat-file", "-t", local) != "commit":
        return [f"main must point at a commit, not {local[:12]}"]
    auth = checks_auth(repo, local)
    if not auth.ok:
        return [f"main {local[:12]} has no passing checks ({auth.how}): run `python3 scripts/gate.py run {local[:12]}`"]
    return []


def tag_update(repo: Repo, remote: str, tag: str, local: str, old: str) -> list[str]:
    if not is_zero(old):
        return [] if old == local else [f"the remote tag {tag} exists; version tags never change or go away"]
    if is_zero(local):
        return [f"cannot delete the absent tag {tag}"]
    try:
        commit = repo.commit(local)
    except GateError:
        return [f"tag {tag} does not point at a commit"]
    errors = []
    for name, auth in (("checks", checks_auth(repo, commit)), ("e2e", e2e_auth(repo, commit))):
        if not auth.ok:
            errors.append(f"tag {tag} at {commit[:12]} has no {name} evidence ({auth.how})")
    if repo.git("config", "--get", f"remote.{remote}.url", check=False).returncode:
        return errors + [f"push version tags through a named remote, not {remote!r}"]
    tracking = repo.ref(f"refs/remotes/{remote}/main")
    if tracking is None:
        return errors + [f"refs/remotes/{remote}/main is unknown: fetch {remote} first"]
    if not repo.is_ancestor(commit, tracking):
        errors.append(f"tag {tag} at {commit[:12]} is not on {remote}/main as last accepted; push main first")
    return errors


# --- Release ------------------------------------------------------------------------------------------------------


def ordinary_env() -> dict[str, str]:
    """The caller's environment for Git and GitHub publication and their authentication, minus overrides."""
    return {k: v for k, v in git_env().items() if k != "CHANGELOG_FILE"}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def github_repo(repo: Repo) -> str:
    url = repo.out("config", "--get", "remote.origin.url")
    found = re.fullmatch(r"(?:git@github\.com:|ssh://git@github\.com/|https://(?:[^@/]+@)?github\.com/)"
                         r"([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+?)(?:\.git)?/?", url)
    if not found:
        raise GateError(f"origin ({url}) is not a GitHub repository")
    return found.group(1)


class Release:
    def __init__(self, sess: Session, tag: str):
        self.sess, self.repo, self.tag = sess, sess.repo, tag
        self.dir = self.repo.state / "release" / tag
        self.state_path = self.repo.state / "release" / f"{tag}.json"
        self.state: dict = json.loads(self.state_path.read_text()) if self.state_path.exists() else {}

    def save(self, **changes) -> None:
        self.state.update(changes)
        write_json(self.state_path, self.state)

    def say(self, text: str) -> None:
        print(f"release {self.tag}: {text}", file=sys.stderr, flush=True)

    def pin(self, rev: str | None) -> str:
        if self.state:
            if self.state.get("schema") != SCHEMA or self.state.get("tag") != self.tag:
                raise GateError(f"{self.state_path} is not a release state of {self.tag}")
            commit = self.state["commit"]
            if rev and self.repo.commit(rev) != commit:
                raise GateError(f"{self.tag} is pinned to {commit}; a retry cannot release another commit")
            return commit
        commit = self.repo.commit(rev or "HEAD")
        self.save(schema=SCHEMA, tag=self.tag, commit=commit, stage="pinned")
        return commit

    def local_tag(self) -> str | None:
        return self.repo.ref(f"refs/tags/{self.tag}")

    def check_local_tag(self, commit: str) -> None:
        local, pinned = self.local_tag(), self.state.get("tag_object")
        if pinned and local != pinned:
            raise GateError(f"the local tag {self.tag} changed from {pinned} to {local}; publication stops")
        if local and self.repo.commit(local) != commit:
            raise GateError(f"the local tag {self.tag} points at another commit than {commit}")

    def release_check(self, path: Path, before_push: bool, env: dict) -> str:
        argv = ["bash", "scripts/release-check.sh", *(["--before-push"] if before_push else []), self.tag]
        done = subprocess.run(argv, cwd=path, env=env, capture_output=True, text=True)
        if done.returncode:
            raise GateError(f"release-check failed: {done.stderr.strip()}")
        return done.stdout

    def remote(self, ref: str) -> str | None:
        listed = self.repo.out("ls-remote", "origin", ref, env=ordinary_env())
        found = [line.split("\t") for line in listed.splitlines()]
        exact = [oid for oid, name in found if name == ref]
        return exact[0] if exact else None

    def publish_main(self, commit: str) -> None:
        for attempt in (1, 2):
            remote = self.remote("refs/heads/main")
            if remote:
                self.repo.git("fetch", "--quiet", "origin", "main", env=ordinary_env())
                if self.repo.is_ancestor(commit, remote):
                    self.say(f"origin main {remote[:12]} already contains {commit[:12]}")
                    return
                if not self.repo.is_ancestor(remote, commit):
                    raise GateError(f"origin main {remote[:12]} diverged from {commit[:12]}; nothing is pushed")
            pushed = self.repo.git("push", "--quiet", "origin", f"{commit}:refs/heads/main", check=False,
                                   env=ordinary_env())
            if pushed.returncode == 0:
                self.repo.git("fetch", "--quiet", "origin", "main", env=ordinary_env())
                after = self.remote("refs/heads/main")
                if not after or not self.repo.is_ancestor(commit, after):
                    raise GateError(f"origin main is {after} after the push, without {commit[:12]}")
                self.say(f"pushed {commit[:12]} to origin main")
                return
            self.say(f"the push of main was rejected (attempt {attempt}): {pushed.stderr.decode().strip()}")
        raise GateError("origin main still refuses the release commit after one refresh")

    def make_tag(self, commit: str) -> str:
        self.check_local_tag(commit)
        if self.local_tag() is None:
            if self.state.get("tag_object"):
                raise GateError(f"the local tag {self.tag} is gone; fetch it, it must stay {self.state['tag_object']}")
            self.repo.git("tag", "-a", self.tag, "-m", self.tag, commit)
        obj = self.local_tag()
        self.check_local_tag(commit)
        self.save(tag_object=obj)
        return obj

    def package(self, path: Path, commit: str, obj: str) -> Path:
        name = f"cicada-skill-{self.tag}.tar.gz"
        work = Path(tempfile.mkdtemp(prefix="package.", dir=self.repo.state))
        try:
            done = subprocess.run(["python3", "-I", "-B", "scripts/package-skill.py", self.tag, str(work)], cwd=path,
                                  env=controlled_env(), capture_output=True, text=True)
            if done.returncode:
                raise GateError(f"packaging the skill failed: {done.stderr.strip()}")
            if self.local_tag() != obj:
                raise GateError(f"the local tag {self.tag} changed while packaging")
            data = (work / name).read_bytes()
        finally:
            for item in work.iterdir():
                item.unlink()
            work.rmdir()
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            members = {m.name: m for m in archive.getmembers()}

            def read(member: str) -> bytes:
                if member not in members or not members[member].isfile():
                    raise GateError(f"the skill archive has no {member}")
                return archive.extractfile(members[member]).read()

            source = json.loads(read("cicada/SOURCE.json"))
            if (source.get("tag"), source.get("commit"), source.get("version")) != (self.tag, commit, self.tag[1:]):
                raise GateError(f"the skill archive names {source}, not {self.tag} at {commit}")
            for inside, repo_path in (("cicada/SKILL.md", "skills/cicada/SKILL.md"),
                                      ("cicada/scripts/cicada_source.py", "skills/cicada/scripts/cicada_source.py")):
                if read(inside) != self.repo.show(self.repo.blob(commit, repo_path) or "missing"):
                    raise GateError(f"{inside} in the archive differs from {repo_path} at {commit}")
        digest = sha256(data)
        if self.state.get("asset_sha256") not in (None, digest):
            raise GateError(f"the skill archive now hashes to {digest}, not {self.state['asset_sha256']}")
        target = self.dir / name
        atomic_write(target, data)
        self.save(asset_sha256=digest)
        return target

    def push_tag(self, obj: str) -> None:
        remote = self.remote(f"refs/tags/{self.tag}")
        if remote == obj:
            return
        if remote is not None:
            raise GateError(f"origin has tag {self.tag} as {remote}, not {obj}; it is never replaced")
        if self.local_tag() != obj:
            raise GateError(f"the local tag {self.tag} changed before its push")
        self.repo.git("push", "--quiet", "origin", f"{obj}:refs/tags/{self.tag}", env=ordinary_env())
        if self.remote(f"refs/tags/{self.tag}") != obj:
            raise GateError(f"origin's tag {self.tag} is not {obj} after the push")
        self.say(f"pushed tag object {obj[:12]}")

    # GitHub, through gh with an explicit --repo.

    def gh(self, *args, check=True) -> subprocess.CompletedProcess:
        done = subprocess.run(["gh", *args], capture_output=True, env=ordinary_env())
        if check and done.returncode:
            raise GateError(f"gh {args[0]} {args[1] if len(args) > 1 else ''}: {done.stderr.decode().strip()}")
        return done

    def releases(self, slug: str) -> list[dict]:
        done = self.gh("api", "--paginate", f"repos/{slug}/releases", "--jq",
                       ".[] | {id, tag_name, name, draft, body, assets: [.assets[] | {id, name, digest}]}")
        found = [json.loads(line) for line in done.stdout.decode().splitlines() if line.strip()]
        return [r for r in found if self.tag in (r.get("tag_name"), r.get("name"))]

    def one_release(self, slug: str) -> dict | None:
        found = self.releases(slug)
        if len(found) > 1:
            raise GateError(f"{len(found)} GitHub releases match {self.tag}; resolve them by hand")
        return found[0] if found else None

    def asset_matches(self, slug: str, asset: dict, digest: str) -> bool:
        if asset.get("digest"):
            return asset["digest"] == f"sha256:{digest}"
        done = self.gh("api", f"repos/{slug}/releases/assets/{asset['id']}", "-H", "Accept: application/octet-stream")
        return sha256(done.stdout) == digest

    def verify_draft(self, slug: str, found: dict, notes: str, asset: Path) -> bool:
        """True when the draft matches with its asset, False when only the asset is missing; GateError otherwise."""
        normal = lambda text: (text or "").replace("\r\n", "\n").strip("\n")
        if found.get("tag_name") != self.tag or found.get("name") != self.tag or normal(found.get("body")) != normal(notes):
            raise GateError(f"draft release {found.get('id')} differs from {self.tag}'s tag, title or notes; left as is")
        assets = found.get("assets") or []
        if any(a.get("name") != asset.name for a in assets) or len(assets) > 1:
            raise GateError(f"draft release {found.get('id')} holds other assets; left as is")
        if not assets:
            return False
        if not self.asset_matches(slug, assets[0], self.state["asset_sha256"]):
            raise GateError(f"draft release {found.get('id')} holds another {asset.name}; left as is")
        return True

    def publish_github(self, obj: str, notes_file: Path, notes: str, asset: Path) -> None:
        slug = github_repo(self.repo)
        found = self.one_release(slug)
        if found and not found.get("draft"):
            if found.get("tag_name") == self.tag and self.remote(f"refs/tags/{self.tag}") == obj:
                self.say("already published; the GitHub release is left untouched")
                self.save(stage="published")
                return
            raise GateError(f"a published GitHub release {found.get('id')} matches {self.tag} for another tag")
        if found is None:
            created = self.gh("release", "create", self.tag, str(asset), "--draft", "--verify-tag", "--title",
                              self.tag, "--notes-file", str(notes_file), "--repo", slug, check=False)
            found = self.one_release(slug)  # also after a failure: the draft may exist anyway
            if found is None:
                raise GateError(f"creating the draft failed: {created.stderr.decode().strip()}")
            if found.get("draft") is not True:
                raise GateError(f"release {found.get('id')} is not a draft after creation; inspect it by hand")
        if not self.verify_draft(slug, found, notes, asset):
            self.gh("release", "upload", self.tag, str(asset), "--repo", slug, check=False)
            found = self.one_release(slug)
            if found is None or found.get("draft") is not True or not self.verify_draft(slug, found, notes, asset):
                raise GateError("the draft has no matching skill archive after the upload")
        self.save(stage="draft-verified")
        if self.remote(f"refs/tags/{self.tag}") != obj:
            raise GateError(f"origin's tag {self.tag} changed before publication")
        self.gh("api", "-X", "PATCH", f"repos/{slug}/releases/{found['id']}", "-F", "draft=false", check=False)
        after = self.one_release(slug)
        if not after or after.get("draft") or after.get("tag_name") != self.tag:
            raise GateError(f"the release of {self.tag} is not published; run the release again to resume")
        if self.remote(f"refs/tags/{self.tag}") != obj:
            raise GateError(f"origin's tag {self.tag} is not {obj} after publication")
        self.save(stage="published")
        self.say("published")

    def run(self, rev: str | None) -> None:
        commit = self.pin(rev)
        self.say(f"pinned to {commit}")
        ensure_runner(self.repo, commit)
        self.check_local_tag(commit)
        path = prepare_owned(self.sess, commit)
        policy_blob, policy, _ = identity(self.repo, commit)
        notes = self.release_check(path, True, command_env(policy, pg_url()))
        if self.state.get("notes_sha256") not in (None, sha256(notes.encode())):
            raise GateError("the release notes differ from the first attempt's")
        notes_file = self.dir / "notes.md"
        atomic_write(notes_file, notes.encode())
        self.save(notes_sha256=sha256(notes.encode()))
        ensure_checks(self.sess, commit, False)
        ensure_e2e(self.sess, commit, False)
        if not (checks_auth(self.repo, commit).ok and e2e_auth(self.repo, commit).ok):
            raise GateError(f"{commit} has no valid checks and e2e evidence")
        self.publish_main(commit)
        self.save(stage="main-published")
        path = prepare_owned(self.sess, commit)
        if self.release_check(path, False, ordinary_env()) != notes:
            raise GateError("release-check printed other notes after main was published")
        obj = self.make_tag(commit)
        remote = self.remote(f"refs/tags/{self.tag}")
        if remote is not None and remote != obj:
            raise GateError(f"origin has tag {self.tag} as {remote}, not {obj}; it is never replaced")
        asset = self.package(path, commit, obj)
        self.push_tag(obj)
        self.save(stage="tag-pushed")
        self.publish_github(obj, notes_file, notes, asset)


# --- Command line -------------------------------------------------------------------------------------------------


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="action", required=True)
    checks = sub.add_parser("checks", help="run a policy in the current directory and record nothing")
    checks.add_argument("--scope", choices=KINDS, required=True)
    run = sub.add_parser("run", help="check a commit in the owned worktree and record the evidence")
    run.add_argument("rev", nargs="?", default="HEAD")
    run.add_argument("--e2e", action="store_true", help="also the Docker end-to-end suite")
    run.add_argument("--force", action="store_true", help="run again although evidence exists")
    status = sub.add_parser("status", help="the evidence of a commit")
    status.add_argument("rev", nargs="?", default="HEAD")
    sub.add_parser("recover", help="clear an abandoned run once its processes are gone")
    sub.add_parser("install-hook", help="install the pre-push gate")
    hook = sub.add_parser("pre-push", help="the pre-push hook")
    hook.add_argument("remote")
    hook.add_argument("url", nargs="?")
    release = sub.add_parser("release", help="publish a checked release")
    release.add_argument("tag")
    release.add_argument("rev", nargs="?")
    args = parser.parse_args(argv)

    try:
        if args.action == "checks":
            policy = parse_policy(Path(POLICY_PATH).read_bytes())
            return run_checks(policy, args.scope, Path.cwd())
        repo = Repo(Path.cwd())
        if args.action == "status":
            print("\n".join(status_lines(repo, repo.commit(args.rev))))
        elif args.action == "recover":
            print(recover(repo))
        elif args.action == "install-hook":
            print(f"installed {install_hook(repo)}")
        elif args.action == "pre-push":
            errors = pre_push(repo, args.remote, sys.stdin.read().splitlines())
            if errors:
                print("gate: push refused:\n  " + "\n  ".join(errors), file=sys.stderr)
                return 1
        elif args.action == "run":
            install_signals()
            commit = repo.commit(args.rev)
            ensure_runner(repo, commit)
            with session(repo, "run") as sess:
                ensure_checks(sess, commit, args.force)
                if args.e2e:
                    ensure_e2e(sess, commit, args.force)
            print("\n".join(status_lines(repo, commit)))
        elif args.action == "release":
            if not VERSION_TAG.fullmatch(args.tag):
                raise GateError(f"'{args.tag}' is not a version tag such as v1.2.3")
            install_signals()
            with session(repo, f"release {args.tag}") as sess:
                Release(sess, args.tag).run(args.rev)
        return 0
    except GateError as error:
        print(f"gate: {error}", file=sys.stderr)
        return 1
    except Uncertain as error:
        print(f"gate: {error}; work may still run, so the gate stays blocked until `gate.py recover`", file=sys.stderr)
        return 1
    except Interrupted as error:
        print(f"gate: interrupted by {error}; no pass was recorded. Check that nothing still runs, then run "
              "`gate.py recover`", file=sys.stderr)
        return 130


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
