#!/usr/bin/env python3
"""Packages the consumer skill of a release tag (ADR 0041).

    python3 scripts/package-skill.py vX.Y.Z OUTPUT_DIR

Run it in the repository. It reads the tag's commit from Git, never the working tree, and writes
OUTPUT_DIR/cicada-skill-vX.Y.Z.tar.gz with cicada/SKILL.md, cicada/scripts/cicada_source.py and cicada/SOURCE.json
(repository, tag, commit, version). It fails, naming the problem, and writes nothing when the tag is malformed or
missing, the workspace version or the skill's `metadata.cicada-version` differs from the tag, a file is missing, or the
archive does not read back as exactly those files. The archive has fixed times, owners and modes, so packaging one
commit again gives the same bytes. Python 3.11 or newer, standard library only.
"""

from __future__ import annotations

import gzip
import io
import json
import re
import subprocess
import sys
import tarfile
import tomllib
from pathlib import Path, PurePosixPath

SKILL_FILES = {  # path in the repository -> path in the archive, mode
    "skills/cicada/SKILL.md": ("cicada/SKILL.md", 0o644),
    "skills/cicada/scripts/cicada_source.py": ("cicada/scripts/cicada_source.py", 0o755),
}
DIRECTORIES = ("cicada", "cicada/scripts")
SOURCE = "cicada/SOURCE.json"


class PackageError(Exception):
    """The tag cannot be packaged."""


def git(*args: str) -> bytes:
    done = subprocess.run(["git", *args], capture_output=True)
    if done.returncode != 0:
        raise PackageError(f"git {' '.join(args)}: {done.stderr.decode(errors='replace').strip()}")
    return done.stdout


def skill_version(text: str) -> str | None:
    """`metadata.cicada-version` of a SKILL.md frontmatter."""
    lines = text.splitlines()
    if not lines or lines[0] != "---":
        return None
    in_metadata = False
    for line in lines[1:]:
        if line == "---":
            return None
        if not line.startswith(" "):
            in_metadata = line.strip() == "metadata:"
        elif in_metadata and line.strip().startswith("cicada-version:"):
            return line.split(":", 1)[1].strip().strip("\"'")
    return None


def read_tag(tag: str) -> tuple[str, dict[str, bytes], dict]:
    """The tag's commit, the skill files at it, and SOURCE.json's fields."""
    if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        raise PackageError(f"'{tag}' is not a version tag such as v1.2.3")
    version = tag[1:]
    try:
        commit = git("rev-parse", "--verify", "--quiet", f"refs/tags/{tag}^{{commit}}").decode().strip()
    except PackageError:
        raise PackageError(f"there is no tag {tag}") from None

    def show(path: str) -> bytes:
        try:
            return git("cat-file", "blob", f"{commit}:{path}")
        except PackageError:
            raise PackageError(f"{path} is missing at {tag} ({commit})") from None

    workspace = tomllib.loads(show("Cargo.toml").decode()).get("workspace", {}).get("package", {})
    if workspace.get("version") != version:
        raise PackageError(f"the tag is {tag}, but the workspace version is {workspace.get('version')}")
    repository = workspace.get("repository")
    if not repository:
        raise PackageError("Cargo.toml has no [workspace.package] repository")
    files = {path: show(path) for path in SKILL_FILES}
    found = skill_version(files["skills/cicada/SKILL.md"].decode())
    if found != version:
        raise PackageError(f"the tag is {tag}, but SKILL.md says cicada-version {found}")
    source = {"repository": repository, "tag": tag, "commit": commit, "version": version}
    return commit, files, source


def contents(files: dict[str, bytes], source: dict) -> dict[str, tuple[bytes | None, int]]:
    """Every member of the archive in order: its bytes (None for a folder) and mode."""
    members: dict[str, tuple[bytes | None, int]] = {name: (None, 0o755) for name in DIRECTORIES}
    for path, (name, mode) in SKILL_FILES.items():
        members[name] = (files[path], mode)
    members[SOURCE] = ((json.dumps(source, indent=2, sort_keys=True) + "\n").encode(), 0o644)
    return dict(sorted(members.items()))


def build(members: dict[str, tuple[bytes | None, int]]) -> bytes:
    tar = io.BytesIO()
    with tarfile.open(fileobj=tar, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for name, (data, mode) in members.items():
            info = tarfile.TarInfo(name)
            info.mode, info.mtime, info.uid, info.gid, info.uname, info.gname = mode, 0, 0, 0, "", ""
            if data is None:
                info.type = tarfile.DIRTYPE
                archive.addfile(info)
            else:
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
    out = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=out, mtime=0, compresslevel=9) as zipped:
        zipped.write(tar.getvalue())
    return out.getvalue()


def safe(name: str) -> bool:
    path = PurePosixPath(name)
    return bool(name) and not path.is_absolute() and ".." not in path.parts and path.parts[0] == "cicada"


def validate(archive: bytes, members: dict[str, tuple[bytes | None, int]]) -> None:
    """Reads the archive back: exactly the expected members, each safe, a file or a folder, with its bytes."""
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
        found = tar.getmembers()
        names = [m.name for m in found]
        for name in names:
            if not safe(name):
                raise PackageError(f"the archive holds an unsafe path {name}")
        if names != list(members):
            raise PackageError(f"the archive holds {names}, not {list(members)}")
        for member in found:
            data, mode = members[member.name]
            if (member.isdir(), member.isfile()) != (data is None, data is not None) or member.mode != mode:
                raise PackageError(f"{member.name} has the wrong type or mode")
            if data is not None and tar.extractfile(member).read() != data:
                raise PackageError(f"{member.name} does not read back as written")


def package(tag: str, output: Path) -> Path:
    _, files, source = read_tag(tag)
    members = contents(files, source)
    archive = build(members)
    validate(archive, members)
    output.mkdir(parents=True, exist_ok=True)
    target = output / f"cicada-skill-{tag}.tar.gz"
    partial = target.with_name(target.name + ".partial")
    partial.write_bytes(archive)
    partial.replace(target)
    return target


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: scripts/package-skill.py vX.Y.Z OUTPUT_DIR", file=sys.stderr)
        return 2
    try:
        print(package(argv[0], Path(argv[1])))
    except PackageError as error:
        print(f"package-skill: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
