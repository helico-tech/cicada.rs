"""Deterministic release archives read from a version tag: the helpers that scripts/package-skill.py (ADR 0041) and
scripts/package-fixtures.py (ADR 0047) share. Each packager loads this file by its path next to itself, as it runs
with `python3 -I`, which keeps the script's folder off sys.path. Python 3.11 or newer, standard library only.
"""

from __future__ import annotations

import gzip
import io
import json
import re
import subprocess
import tarfile
import tomllib
from pathlib import Path, PurePosixPath

Members = dict[str, tuple[bytes | None, int]]  # name in the archive -> its bytes (None for a folder) and mode


class PackageError(Exception):
    """The tag cannot be packaged."""


def git(*args: str) -> bytes:
    done = subprocess.run(["git", *args], capture_output=True)
    if done.returncode != 0:
        raise PackageError(f"git {' '.join(args)}: {done.stderr.decode(errors='replace').strip()}")
    return done.stdout


class Tag:
    """A version tag: its commit, the files at it, and the workspace's version and repository, checked."""

    def __init__(self, tag: str):
        if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
            raise PackageError(f"'{tag}' is not a version tag such as v1.2.3")
        self.tag, self.version = tag, tag[1:]
        try:
            self.commit = git("rev-parse", "--verify", "--quiet", f"refs/tags/{tag}^{{commit}}").decode().strip()
        except PackageError:
            raise PackageError(f"there is no tag {tag}") from None
        workspace = tomllib.loads(self.show("Cargo.toml").decode()).get("workspace", {}).get("package", {})
        if workspace.get("version") != self.version:
            raise PackageError(f"the tag is {tag}, but the workspace version is {workspace.get('version')}")
        self.repository = workspace.get("repository")
        if not self.repository:
            raise PackageError("Cargo.toml has no [workspace.package] repository")

    def show(self, path: str) -> bytes:
        try:
            return git("cat-file", "blob", f"{self.commit}:{path}")
        except PackageError:
            raise PackageError(f"{path} is missing at {self.tag} ({self.commit})") from None

    def source(self) -> bytes:
        """SOURCE.json: the repository, tag, commit and version of the archive."""
        value = {"repository": self.repository, "tag": self.tag, "commit": self.commit, "version": self.version}
        return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def build(members: Members) -> bytes:
    """A gzipped USTAR archive of the members in the given order, with fixed times, owners and gzip header."""
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


def safe(name: str, root: str) -> bool:
    path = PurePosixPath(name)
    return bool(name) and not path.is_absolute() and ".." not in path.parts and path.parts[0] == root


def validate(archive: bytes, members: Members, root: str) -> None:
    """Reads the archive back: exactly the expected members, each safe, a file or a folder, with its bytes."""
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
        found = tar.getmembers()
        names = [m.name for m in found]
        for name in names:
            if not safe(name, root):
                raise PackageError(f"the archive holds an unsafe path {name}")
        if names != list(members):
            raise PackageError(f"the archive holds {names}, not {list(members)}")
        for member in found:
            data, mode = members[member.name]
            if (member.isdir(), member.isfile()) != (data is None, data is not None) or member.mode != mode:
                raise PackageError(f"{member.name} has the wrong type or mode")
            if data is not None and tar.extractfile(member).read() != data:
                raise PackageError(f"{member.name} does not read back as written")


def write(output: Path, name: str, archive: bytes) -> Path:
    """Writes the archive as OUTPUT/name through a partial file, so a reader never sees half of it."""
    output.mkdir(parents=True, exist_ok=True)
    target = output / name
    partial = target.with_name(target.name + ".partial")
    partial.write_bytes(archive)
    partial.replace(target)
    return target
