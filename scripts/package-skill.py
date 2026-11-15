#!/usr/bin/env python3
"""Packages the consumer skill of a release tag (ADR 0041).

    python3 scripts/package-skill.py vX.Y.Z OUTPUT_DIR

Run it in the repository. It reads the tag's commit from Git, never the working tree, and writes
OUTPUT_DIR/cicada-skill-vX.Y.Z.tar.gz with cicada/SKILL.md, cicada/scripts/cicada_source.py and cicada/SOURCE.json
(repository, tag, commit, version). It fails, naming the problem, and writes nothing when the tag is malformed or
missing, the workspace version or the skill's `metadata.cicada-version` differs from the tag, a file is missing, or the
archive does not read back as exactly those files. The archive has fixed times, owners and modes, so packaging one
commit again gives the same bytes. Python 3.11 or newer, standard library only; it shares scripts/release_archive.py
with the fixture packager.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

_helpers = Path(__file__).resolve().with_name("release_archive.py")  # by path: `python3 -I` keeps this folder off sys.path
_spec = importlib.util.spec_from_file_location("release_archive", _helpers)
release_archive = importlib.util.module_from_spec(_spec)
sys.dont_write_bytecode = True  # never write into the checkout, whatever the interpreter flags
_spec.loader.exec_module(release_archive)
PackageError = release_archive.PackageError
build = release_archive.build

SKILL_FILES = {  # path in the repository -> path in the archive, mode
    "skills/cicada/SKILL.md": ("cicada/SKILL.md", 0o644),
    "skills/cicada/scripts/cicada_source.py": ("cicada/scripts/cicada_source.py", 0o755),
}
DIRECTORIES = ("cicada", "cicada/scripts")
SOURCE = "cicada/SOURCE.json"
ROOT = "cicada"


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


def read_tag(tag: str):
    """The tag, checked, and the skill files at its commit."""
    found = release_archive.Tag(tag)
    files = {path: found.show(path) for path in SKILL_FILES}
    version = skill_version(files["skills/cicada/SKILL.md"].decode())
    if version != found.version:
        raise PackageError(f"the tag is {tag}, but SKILL.md says cicada-version {version}")
    return found, files


def contents(found, files: dict[str, bytes]) -> dict[str, tuple[bytes | None, int]]:
    """Every member of the archive in order: its bytes (None for a folder) and mode."""
    members: dict[str, tuple[bytes | None, int]] = {name: (None, 0o755) for name in DIRECTORIES}
    for path, (name, mode) in SKILL_FILES.items():
        members[name] = (files[path], mode)
    members[SOURCE] = (found.source(), 0o644)
    return dict(sorted(members.items()))


def validate(archive: bytes, members: dict[str, tuple[bytes | None, int]]) -> None:
    """Reads the archive back: exactly the expected members, each safe, a file or a folder, with its bytes."""
    release_archive.validate(archive, members, ROOT)


def package(tag: str, output: Path) -> Path:
    found, files = read_tag(tag)
    members = contents(found, files)
    archive = build(members)
    validate(archive, members)
    return release_archive.write(output, f"cicada-skill-{tag}.tar.gz", archive)


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
