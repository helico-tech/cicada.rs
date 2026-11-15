#!/usr/bin/env python3
"""Packages the conformance fixtures of a release tag (ADR 0047).

    python3 scripts/package-fixtures.py vX.Y.Z OUTPUT_DIR

Run it in the repository. It reads the tag's commit from Git, never the working tree, and writes
OUTPUT_DIR/cicada-fixtures-vX.Y.Z.tar.gz with cicada-fixtures/ and in it canonical.json, contracts.json, leases.json,
status.json and SOURCE.json (repository, tag, commit, version). It fails, naming the problem, and writes nothing when
the tag is malformed or missing, the workspace version differs from the tag, `fixtures/` at the tag holds another set
of entries, an entry that is not a regular file (a folder, a link or a submodule) or a file that is not a JSON object
with `format: 1`, or when the archive does not read back as exactly those files. The archive has fixed times, owners
and modes, so packaging one commit again gives the same bytes. Python 3.11 or newer, standard library only; it shares
scripts/release_archive.py with the skill packager.
"""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

_helpers = Path(__file__).resolve().with_name("release_archive.py")  # by path: `python3 -I` keeps this folder off sys.path
_spec = importlib.util.spec_from_file_location("release_archive", _helpers)
release_archive = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(release_archive)
PackageError = release_archive.PackageError

FIXTURES = ("canonical.json", "contracts.json", "leases.json", "status.json")
FORMAT = 1
ROOT = "cicada-fixtures"
REGULAR = ("100644", "100755")  # Git's modes of a regular file


def read_fixtures(found) -> dict[str, bytes]:
    """The four fixture files at the tag's commit, checked: exactly those entries, regular files, format 1."""
    folder = entries_of(found.commit, "--", "fixtures")
    if not folder:
        raise PackageError(f"fixtures/ is missing at {found.tag} ({found.commit})")
    if folder["fixtures"][1] != "tree":
        raise PackageError(f"fixtures at {found.tag} is not a folder")
    entries = entries_of(f"{found.commit}:fixtures")
    if sorted(entries) != sorted(FIXTURES):
        raise PackageError(f"fixtures/ at {found.tag} holds {sorted(entries)}, not {list(FIXTURES)}")
    files = {}
    for name in FIXTURES:
        mode, kind, oid = entries[name]
        if kind != "blob" or mode not in REGULAR:
            raise PackageError(f"fixtures/{name} at {found.tag} is not a regular file (mode {mode})")
        data = release_archive.git("cat-file", "blob", oid)
        try:
            value = json.loads(data)
        except ValueError as error:
            raise PackageError(f"fixtures/{name} at {found.tag} is not JSON: {error}") from None
        if not isinstance(value, dict) or type(value.get("format")) is not int or value["format"] != FORMAT:
            raise PackageError(f"fixtures/{name} at {found.tag} is not a fixture of format {FORMAT}")
        files[name] = data
    return files


def entries_of(*args: str) -> dict[str, tuple[str, str, str]]:
    """`git ls-tree` of the arguments: each name with its mode, kind and object."""
    entries = {}
    for line in filter(None, release_archive.git("ls-tree", "-z", *args).decode().split("\0")):
        meta, name = line.split("\t", 1)
        mode, kind, oid = meta.split(" ")
        entries[name] = (mode, kind, oid)
    return entries


def contents(found, files: dict[str, bytes]) -> dict[str, tuple[bytes | None, int]]:
    """Every member of the archive in order: its bytes (None for the folder) and mode."""
    members: dict[str, tuple[bytes | None, int]] = {ROOT: (None, 0o755)}
    for name in FIXTURES:
        members[f"{ROOT}/{name}"] = (files[name], 0o644)
    members[f"{ROOT}/SOURCE.json"] = (found.source(), 0o644)
    return dict(sorted(members.items()))


def package(tag: str, output: Path) -> Path:
    found = release_archive.Tag(tag)
    members = contents(found, read_fixtures(found))
    archive = release_archive.build(members)
    release_archive.validate(archive, members, ROOT)
    return release_archive.write(output, f"cicada-fixtures-{tag}.tar.gz", archive)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: scripts/package-fixtures.py vX.Y.Z OUTPUT_DIR", file=sys.stderr)
        return 2
    try:
        print(package(argv[0], Path(argv[1])))
    except PackageError as error:
        print(f"package-fixtures: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
