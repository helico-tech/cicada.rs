#!/usr/bin/env python3
"""Find the Cicada source a Rust project resolves to, and print one JSON report.

    python3 cicada_source.py [--manifest-path Cargo.toml] [--target TRIPLE]
                             [--features F]... [--all-features] [--no-default-features]
                             [--metadata FILE] [--timeout SECONDS]

Read-only. It runs `rustc -vV` (only when no target is given) and
`cargo metadata --format-version 1 --locked --offline --filter-platform TARGET` for the selected manifest, with
RUSTUP_AUTO_INSTALL=0, and `git` read commands in the source roots it finds. It never fetches, installs a tool or a
toolchain, writes a lockfile, retries without --locked/--offline, or changes the project or the installed skill.
--metadata FILE reads saved `cargo metadata` output instead of running Cargo (for tests).

The report's `status`: ok (one source with docs), none (no Cicada crate in the selection), mixed (more than one
version or source), self (the project is the Cicada workspace), no-docs (one source whose docs tree is not there) or
unavailable (no lockfile, a tool or toolchain missing, Cargo failed or timed out; see `reason` and `diagnostic`).
Python 3.11 or newer, standard library only.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

PACKAGES = (
    "cicada",
    "cicada-test",
    "cicada-axum",
    "cicada-client",
    "cicada-otel",
    "cicada-actors",
    "cicada-app",
    "cicada-containers",
)
SKILL_DIR = Path(__file__).resolve().parent.parent
DEFAULT_TIMEOUT = 120.0
GIT_TIMEOUT = 20.0
FETCH_HINT = (
    "If Cargo says a locked package is not downloaded, `cargo fetch --locked` downloads exactly the locked sources. "
    "This helper never runs it; ask before you do."
)


class Unavailable(Exception):
    """Discovery cannot go on; the report says why."""

    def __init__(self, reason: str, diagnostic: str, hint: str | None = None):
        super().__init__(reason)
        self.reason = reason
        self.diagnostic = diagnostic
        self.hint = hint


def tool_env() -> dict[str, str]:
    """The environment of Cargo and rustc: rustup installs no toolchain, and nothing asks for input."""
    env = dict(os.environ)
    env["RUSTUP_AUTO_INSTALL"] = "0"
    env["CARGO_TERM_COLOR"] = "never"
    env["GIT_TERMINAL_PROMPT"] = "0"
    return env


def tail(text: str, limit: int = 600) -> str:
    text = text.strip()
    return text if len(text) <= limit else "…" + text[-limit:]


def run_tool(args: list[str], cwd: Path, timeout: float, reason: str, hint: str | None = None) -> str:
    """Runs a Cargo or rustc command once and returns its stdout, or raises Unavailable."""
    try:
        done = subprocess.run(
            args, cwd=cwd, env=tool_env(), capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL
        )
    except FileNotFoundError:
        raise Unavailable("missing-tool", f"`{args[0]}` was not found on PATH") from None
    except subprocess.TimeoutExpired:
        raise Unavailable("timeout", f"`{' '.join(args[:2])}` did not finish within {timeout:g} s") from None
    if done.returncode != 0:
        raise Unavailable(reason, tail(done.stderr) or f"`{args[0]}` exited with {done.returncode}", hint)
    return done.stdout


def git(root: Path, *args: str) -> str | None:
    """A read-only git command in `root`; None when git or the repository is not there."""
    try:
        done = subprocess.run(
            ["git", "--no-optional-locks", "-C", str(root), *args],
            env=tool_env(),
            capture_output=True,
            text=True,
            timeout=GIT_TIMEOUT,
            stdin=subprocess.DEVNULL,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    return done.stdout if done.returncode == 0 else None


def read_skill_version() -> str | None:
    """`metadata.cicada-version` of the SKILL.md beside this script's folder."""
    try:
        text = (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8")
    except OSError:
        return None
    return frontmatter_version(text)


def frontmatter_version(text: str) -> str | None:
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return None
    in_metadata = False
    for line in lines[1:]:
        if line.strip() == "---":
            break
        if not line.startswith((" ", "\t")):
            in_metadata = line.split("#")[0].strip() == "metadata:"
            continue
        if in_metadata:
            key, _, value = line.strip().partition(":")
            if key == "cicada-version":
                return value.strip().strip("\"'") or None
    return None


def read_skill_source() -> dict | None:
    """SOURCE.json of a released skill archive: repository, tag, commit and version. None for a copied skill."""
    try:
        data = json.loads((SKILL_DIR / "SOURCE.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return data if isinstance(data, dict) else None


def parse_version(text: str | None) -> tuple[tuple[int, int, int], str] | None:
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?", text or "")
    if not match:
        return None
    return (int(match[1]), int(match[2]), int(match[3])), match[4] or ""


def version_relation(project: str | None, skill: str | None) -> str:
    """older, equal or newer: the project's version against the skill's; unknown when either cannot be compared."""
    a, b = parse_version(project), parse_version(skill)
    if a is None or b is None:
        return "unknown"
    if a[0] != b[0]:
        return "older" if a[0] < b[0] else "newer"
    if a[1] == b[1]:
        return "equal"
    if not a[1] or not b[1]:  # a release is newer than its pre-releases
        return "newer" if not a[1] else "older"
    return "unknown"


def verified_root(manifest_path: str) -> Path | None:
    """The Cicada tree of a package's manifest (`<root>/crates/<name>/Cargo.toml`) when it holds the core and docs."""
    manifest = Path(manifest_path)
    if len(manifest.parents) < 3 or manifest.parents[1].name != "crates":
        return None
    root = manifest.parents[2]
    if (root / "crates" / "cicada" / "Cargo.toml").is_file() and (root / "docs" / "architecture.md").is_file():
        return root.resolve()
    return None


def parse_git_source(source: str) -> dict:
    """The parts of a Cargo git source id: `git+URL?tag=v1#<full revision>`."""
    rest = source[len("git+") :]
    rest, _, revision = rest.partition("#")
    parts = urlsplit(rest)
    query = parse_qs(parts.query)
    selector = None
    for kind in ("tag", "branch", "rev"):
        if kind in query:
            selector = {"kind": kind, "value": query[kind][0]}
    url = parts._replace(query="", fragment="").geturl()
    return {"url": url, "selector": selector, "revision": revision or None}


def probe_git(root: Path) -> dict:
    """HEAD and local changes of the checkout at `root`, read with git. A probe that cannot run or fails leaves its
    field None (unknown): only a successful probe says anything."""
    unknown = {"commit": None, "dirty": None, "git_root": None, "unversioned": None}
    if shutil.which("git") is None:
        return unknown
    top = git(root, "rev-parse", "--show-toplevel")
    if top is None:
        return dict(unknown, unversioned=True)
    head = git(root, "rev-parse", "--verify", "--quiet", "HEAD")
    status = git(root, "status", "--porcelain", "--", ".")
    dirty = None
    if status is not None:
        # Cargo writes .cargo-ok into each git checkout it makes; it is no change of the source.
        changes = [line for line in status.splitlines() if line.strip() and not line.endswith(".cargo-ok")]
        dirty = bool(changes)
    return {
        "commit": (head or "").strip() or None,
        "dirty": dirty,
        "git_root": str(Path(top.strip()).resolve()),
        "unversioned": False,
    }


def group_key(package: dict, root: Path | None) -> tuple:
    source = package.get("source")
    version = package.get("version")
    if source:
        return ("source", source, version)
    if root is not None:
        return ("path", str(root), version)
    # A path package without a verified tree keeps its own manifest folder, so unrelated paths stay apart.
    return ("manifest", str(Path(package["manifest_path"]).parent), version)


def discover(metadata: dict, skill_version: str | None, skill_source: dict | None) -> dict:
    """Classifies the Cicada packages of a filtered resolve graph."""
    resolve = metadata.get("resolve")
    if not isinstance(resolve, dict) or not isinstance(resolve.get("nodes"), list):
        raise Unavailable("bad-metadata", "the metadata has no resolve graph (was it made with --no-deps?)")
    by_id = {p["id"]: p for p in metadata.get("packages", []) if isinstance(p, dict) and "id" in p}
    selected = []
    for node in resolve["nodes"]:
        package = by_id.get(node.get("id"))
        if package is not None and package.get("name") in PACKAGES:
            selected.append(package)
    selected.sort(key=lambda p: (p["name"], p.get("version") or "", p["id"]))

    entries, groups = [], {}
    for package in selected:
        root = verified_root(package["manifest_path"])
        key = group_key(package, root)
        entries.append((package, root, key))
        groups.setdefault(key, {"root": root, "package": package, "names": []})["names"].append(package["name"])
    index_of = {key: index for index, key in enumerate(sorted(groups, key=lambda k: [str(part) for part in k]))}

    skill_commit = (skill_source or {}).get("commit")
    reported = []
    for key, index in sorted(index_of.items(), key=lambda kv: kv[1]):
        entry = groups[key]
        root, package = entry["root"], entry["package"]
        source = package.get("source")
        group = {
            "index": index,
            "kind": "path" if not source else source.split("+", 1)[0],
            "version": package.get("version"),
            "source": source,
            "packages": sorted(entry["names"]),
            "root": str(root) if root else None,
            "manifest_folder": None if root else str(Path(package["manifest_path"]).parent),
            "docs_available": root is not None,
            "url": None,
            "selector": None,
            "revision": None,
            "commit": None,
            "dirty": None,
            "unversioned": None,
            "git_root": None,
            "head_matches_revision": None,
        }
        if source and source.startswith("git+"):
            group.update(parse_git_source(source))
        probe_dir = root or Path(package["manifest_path"]).parent
        if probe_dir.is_dir():
            group.update(probe_git(probe_dir))
        if group["revision"] and group["commit"]:
            group["head_matches_revision"] = group["revision"] == group["commit"]
        # Proven only by a HEAD and a status that were both read: Cargo's locked revision says nothing of the checkout.
        probed = group["commit"] is not None and group["dirty"] is not None
        if not skill_commit or not probed:
            group["revision_match"] = "unknown"
        else:
            group["revision_match"] = "same" if group["commit"] == skill_commit else "different"
        # Local changes, or a checkout whose HEAD is not the revision Cargo locked; None when not probed.
        group["modified"] = (group["dirty"] or group["head_matches_revision"] is False) if probed else None
        group["version_relation"] = version_relation(group["version"], skill_version)
        reported.append(group)

    packages = [
        {
            "name": package["name"],
            "id": package["id"],
            "version": package.get("version"),
            "source": package.get("source"),
            "manifest_path": package["manifest_path"],
            "root": str(root) if root else None,
            "group": index_of[key],
        }
        for package, root, key in entries
    ]

    workspace_root = metadata.get("workspace_root")
    members = set(metadata.get("workspace_members") or [])
    if not reported:
        status = "none"
    elif is_self(selected, members, workspace_root):
        status = "self"
    elif len(reported) > 1:
        status = "mixed"
    elif not reported[0]["docs_available"]:
        status = "no-docs"
    else:
        status = "ok"
    return {"status": status, "packages": packages, "groups": reported, "workspace": workspace_root}


def is_self(selected: list[dict], members: set[str], workspace_root: str | None) -> bool:
    """Whether the core package is a member of the selected workspace, with the verified Cicada tree at its root."""
    if not workspace_root:
        return False
    for package in selected:
        if package["name"] == "cicada" and package["id"] in members:
            root = verified_root(package["manifest_path"])
            if root is not None and root == Path(workspace_root).resolve():
                return True
    return False


def feature_args(options: argparse.Namespace) -> list[str]:
    args = []
    if options.all_features:
        args.append("--all-features")
    if options.no_default_features:
        args.append("--no-default-features")
    for feature in options.features:
        args += ["--features", feature]
    return args


def load_metadata(options: argparse.Namespace) -> tuple[dict, str | None]:
    """Runs Cargo (or reads --metadata) and returns the metadata and the target it was filtered for."""
    if options.metadata:
        try:
            return json.loads(Path(options.metadata).read_text(encoding="utf-8")), options.target
        except (OSError, ValueError) as error:
            raise Unavailable("bad-metadata", f"{options.metadata}: {error}") from None

    if options.manifest_path:
        manifest = Path(options.manifest_path).resolve()
        if not manifest.is_file():
            raise Unavailable("missing-manifest", f"{manifest} is not a file")
        cwd = manifest.parent
    else:
        manifest, cwd = None, Path.cwd()
    manifest_args = ["--manifest-path", str(manifest)] if manifest else []

    located = run_tool(
        ["cargo", "--locked", "--offline", "locate-project", "--workspace", "--message-format", "plain", *manifest_args],
        cwd,
        options.timeout,
        "locate-failed",
    ).strip()
    lock = Path(located).parent / "Cargo.lock"
    if not lock.is_file():
        raise Unavailable(
            "missing-lock",
            f"{lock} does not exist; the helper reads only a locked resolution and never writes one",
        )

    target = options.target
    if not target:
        version = run_tool(["rustc", "-vV"], cwd, options.timeout, "rustc-failed")
        match = re.search(r"^host:\s*(\S+)", version, re.MULTILINE)
        if not match:
            raise Unavailable("rustc-failed", "`rustc -vV` printed no host triple")
        target = match[1]

    args = ["cargo", "metadata", "--format-version", "1", "--locked", "--offline", "--filter-platform", target]
    out = run_tool(
        [*args, *manifest_args, *feature_args(options)], cwd, options.timeout, "metadata-failed", FETCH_HINT
    )
    try:
        return json.loads(out), target
    except ValueError as error:
        raise Unavailable("bad-metadata", f"cargo metadata printed no JSON: {error}") from None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--manifest-path", help="the Cargo.toml to resolve (default: Cargo's own search from here)")
    parser.add_argument("--target", help="the target triple to filter for (default: the host of `rustc -vV`)")
    parser.add_argument("--features", action="append", default=[], help="features to enable; repeatable")
    parser.add_argument("--all-features", action="store_true")
    parser.add_argument("--no-default-features", action="store_true")
    parser.add_argument("--metadata", help="read saved `cargo metadata` JSON instead of running Cargo")
    parser.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT, help="seconds per command (default 120)")
    options = parser.parse_args(argv)

    skill_version, skill_source = read_skill_version(), read_skill_source()
    report = {
        "status": None,
        "target": options.target,
        "features": {
            "all_features": options.all_features,
            "no_default_features": options.no_default_features,
            "features": options.features,
        },
        "note": "Only this target and feature selection was examined.",
        "workspace": None,
        "skill_version": skill_version,
        "skill_source": skill_source,
        "packages": [],
        "groups": [],
    }
    try:
        metadata, target = load_metadata(options)
        report["target"] = target
        report.update(discover(metadata, skill_version, skill_source))
    except Unavailable as error:
        report.update(status="unavailable", reason=error.reason, diagnostic=error.diagnostic)
        if error.hint:
            report["hint"] = error.hint
    json.dump(report, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    if sys.version_info < (3, 11):
        sys.exit("cicada_source.py needs Python 3.11 or newer")
    sys.exit(main())
