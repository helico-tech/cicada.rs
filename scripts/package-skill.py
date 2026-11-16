#!/usr/bin/env python3
"""Packages the consumer skills of a release tag (ADRs 0041 and 0051).

    python3 scripts/package-skill.py vX.Y.Z OUTPUT_DIR

Run it in the repository. It reads the tag's commit from Git, never the working tree, and writes
OUTPUT_DIR/cicada-skill-vX.Y.Z.tar.gz. The tag's scripts/skill-archive-format selects the layout:

- format 2 (the file holds `2` and a newline): cicada/ with SKILL.md, scripts/cicada_source.py and SOURCE.json, and
  actor-design/ with SKILL.md, assets/design-template.md and the same SOURCE.json (repository, tag, commit, version);
- the single-skill layout of earlier releases, only when the tag has neither that file nor skills/actor-design: cicada/
  alone, byte for byte as before.

It fails, naming the problem, and writes nothing when the tag is malformed or missing, the workspace version or a
skill's `metadata.cicada-version` differs from the tag, the format file is malformed or missing beside
skills/actor-design, a file is missing or not a regular file (a link, a folder or a submodule), or the archive does not
read back as exactly those files. The archive has fixed times, owners and modes, so packaging one commit again gives
the same bytes. Python 3.11 or newer, standard library only; it shares scripts/release_archive.py with the fixture
packager.
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

FORMAT_FILE = "scripts/skill-archive-format"  # the layout marker of the tag; not a member of the archive
CICADA_FILES = {  # path in the repository -> path in the archive, mode
    "skills/cicada/SKILL.md": ("cicada/SKILL.md", 0o644),
    "skills/cicada/scripts/cicada_source.py": ("cicada/scripts/cicada_source.py", 0o755),
}
DESIGN_FILES = {
    "skills/actor-design/SKILL.md": ("actor-design/SKILL.md", 0o644),
    "skills/actor-design/assets/design-template.md": ("actor-design/assets/design-template.md", 0o644),
}
LAYOUTS = {  # format -> files, folders, SOURCE.json files
    1: (CICADA_FILES, ("cicada", "cicada/scripts"), ("cicada/SOURCE.json",)),
    2: (
        {**CICADA_FILES, **DESIGN_FILES},
        ("actor-design", "actor-design/assets", "cicada", "cicada/scripts"),
        ("actor-design/SOURCE.json", "cicada/SOURCE.json"),
    ),
}
SKILLS = ("skills/cicada/SKILL.md", "skills/actor-design/SKILL.md")  # each names the release in its metadata
ROOTS = ("actor-design", "cicada")


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


def layout(found) -> int:
    """The archive format the tag selects: its format file, or the single-skill layout when it has neither that file
    nor skills/actor-design."""
    if found.entry(FORMAT_FILE) is None:
        if found.entry("skills/actor-design") is not None:
            raise PackageError(f"{found.tag} has skills/actor-design but no {FORMAT_FILE}")
        return 1
    marker = found.regular(FORMAT_FILE)
    if marker != b"2\n":
        raise PackageError(f"{FORMAT_FILE} at {found.tag} must hold 2 and a newline, holds {marker[:20]!r}")
    return 2


def read_tag(tag: str):
    """The tag, checked, its archive format, and the skill files at its commit."""
    found = release_archive.Tag(tag)
    form = layout(found)
    files = {path: found.regular(path) for path in LAYOUTS[form][0]}
    for path in SKILLS:
        if path in files:
            version = skill_version(files[path].decode())
            if version != found.version:
                raise PackageError(f"the tag is {tag}, but {path} says cicada-version {version}")
    return found, form, files


def contents(found, form: int, files: dict[str, bytes]) -> dict[str, tuple[bytes | None, int]]:
    """Every member of the archive in order: its bytes (None for a folder) and mode."""
    paths, folders, sources = LAYOUTS[form]
    members: dict[str, tuple[bytes | None, int]] = {name: (None, 0o755) for name in folders}
    for path, (name, mode) in paths.items():
        members[name] = (files[path], mode)
    for name in sources:
        members[name] = (found.source(), 0o644)
    return dict(sorted(members.items()))


def validate(archive: bytes, members: dict[str, tuple[bytes | None, int]]) -> None:
    """Reads the archive back: exactly the expected members, each safe, a file or a folder, with its bytes."""
    release_archive.validate(archive, members, ROOTS)


def package(tag: str, output: Path) -> Path:
    found, form, files = read_tag(tag)
    members = contents(found, form, files)
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
