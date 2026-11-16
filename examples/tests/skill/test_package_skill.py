"""Tests of scripts/package-skill.py, run by examples/tests/skill.rs (or `python3 -m unittest` here).

Each test packages a tag of a temporary Git repository that holds the skill of this checkout; none packages a real
Cicada tag, publishes anything or moves a tag.
"""

from __future__ import annotations

import gzip
import hashlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
SCRIPT = REPO / "scripts" / "package-skill.py"
GIT = ["git", "-c", "user.name=test", "-c", "user.email=test@example.test", "-c", "commit.gpgsign=false",
       "-c", "tag.gpgsign=false"]
REPOSITORY = "https://example.test/cicada.rs"
NAMES = ["cicada", "cicada/SKILL.md", "cicada/SOURCE.json", "cicada/scripts", "cicada/scripts/cicada_source.py"]
DESIGN_NAMES = ["actor-design", "actor-design/SKILL.md", "actor-design/SOURCE.json", "actor-design/assets",
                "actor-design/assets/design-template.md"]
MARKER = "scripts/skill-archive-format"


def load_module():
    spec = importlib.util.spec_from_file_location("package_skill", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Packaging(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.repo = self.dir / "repo"
        (self.repo / "skills" / "cicada" / "scripts").mkdir(parents=True)
        self.git("init", "-q")
        self.commit_version("0.17.2")

    def git(self, *args: str) -> str:
        return subprocess.run([*GIT, *args], cwd=self.repo, check=True, capture_output=True, text=True).stdout.strip()

    def write_skill(self, version: str, script: bool = True):
        text = (REPO / "skills" / "cicada" / "SKILL.md").read_text(encoding="utf-8")
        lines = [f"  cicada-version: {version}" if l.startswith("  cicada-version:") else l for l in text.split("\n")]
        (self.repo / "skills" / "cicada" / "SKILL.md").write_text("\n".join(lines), encoding="utf-8")
        helper = self.repo / "skills" / "cicada" / "scripts" / "cicada_source.py"
        if script:
            shutil.copy(REPO / "skills" / "cicada" / "scripts" / "cicada_source.py", helper)
        elif helper.exists():
            helper.unlink()

    def commit_version(self, version: str, skill: str | None = None, script: bool = True) -> str:
        (self.repo / "Cargo.toml").write_text(
            f'[workspace]\nmembers = []\n\n[workspace.package]\nversion = "{version}"\nrepository = "{REPOSITORY}"\n',
            encoding="utf-8",
        )
        self.write_skill(skill or version, script)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", f"version {version}")
        return self.git("rev-parse", "HEAD")

    def write_design(self, version: str, marker: bytes | None = b"2\n", template: bool = True):
        """The actor-design skill of this checkout at `version`, and the format file holding `marker` (None: none)."""
        folder = self.repo / "skills" / "actor-design"
        (folder / "assets").mkdir(parents=True, exist_ok=True)
        text = (REPO / "skills" / "actor-design" / "SKILL.md").read_text(encoding="utf-8")
        lines = [f"  cicada-version: {version}" if l.startswith("  cicada-version:") else l for l in text.split("\n")]
        (folder / "SKILL.md").write_text("\n".join(lines), encoding="utf-8")
        copy = folder / "assets" / "design-template.md"
        copy.unlink(missing_ok=True)
        if template:
            shutil.copy(REPO / "skills" / "actor-design" / "assets" / "design-template.md", copy)
        (self.repo / "scripts").mkdir(exist_ok=True)
        (self.repo / MARKER).unlink(missing_ok=True)
        if marker is not None:
            (self.repo / MARKER).write_bytes(marker)

    def commit_both(self, version: str, design: str | None = None, **kwargs) -> str:
        """A commit of `version` with both skills and the format file; `design` sets actor-design's version."""
        self.write_design(design or version, **kwargs)
        return self.commit_version(version)

    def package(self, tag: str, output: str = "dist") -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, "-I", str(SCRIPT), tag, str(self.dir / output)],
            cwd=self.repo,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def archive(self, tag: str, output: str = "dist") -> bytes:
        done = self.package(tag, output)
        self.assertEqual(done.returncode, 0, done.stderr)
        path = self.dir / output / f"cicada-skill-{tag}.tar.gz"
        self.assertEqual(done.stdout.strip(), str(path))
        return path.read_bytes()

    def test_an_annotated_tag_packages_its_commit_exactly(self):
        commit = self.git("rev-parse", "HEAD")
        self.git("tag", "-a", "v0.17.2", "-m", "v0.17.2")
        self.assertNotEqual(self.git("rev-parse", "v0.17.2"), commit, "an annotated tag is an object of its own")
        data = self.archive("v0.17.2")
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
            members = tar.getmembers()
            self.assertEqual([m.name for m in members], NAMES)
            for member in members:
                self.assertEqual((member.mtime, member.uid, member.gid, member.uname, member.gname), (0, 0, 0, "", ""))
                self.assertFalse(member.name.startswith("/") or ".." in member.name.split("/"))
            read = lambda name: tar.extractfile(name).read()
            self.assertEqual(read("cicada/SKILL.md"), (self.repo / "skills/cicada/SKILL.md").read_bytes())
            self.assertEqual(
                read("cicada/scripts/cicada_source.py"),
                (REPO / "skills/cicada/scripts/cicada_source.py").read_bytes(),
            )
            self.assertEqual(tar.getmember("cicada/scripts/cicada_source.py").mode, 0o755)
            source = json.loads(read("cicada/SOURCE.json"))
        self.assertEqual(source, {"repository": REPOSITORY, "tag": "v0.17.2", "commit": commit, "version": "0.17.2"})
        self.assertNotIn(str(self.dir).encode(), data)

    def test_packaging_again_gives_the_same_bytes_whatever_the_working_tree_holds(self):
        self.git("tag", "v0.17.2")  # a lightweight tag too
        first = self.archive("v0.17.2", "one")
        (self.repo / "skills" / "cicada" / "SKILL.md").write_text("changed, not committed\n", encoding="utf-8")
        (self.repo / "skills" / "cicada" / "notes.md").write_text("untracked\n", encoding="utf-8")
        (self.repo / "Cargo.toml").write_text("[workspace.package]\nversion = \"9.9.9\"\n", encoding="utf-8")
        self.assertEqual(self.archive("v0.17.2", "two"), first)
        self.assertEqual(self.archive("v0.17.2", "two"), first)

    def test_the_unpacked_skill_runs_and_knows_its_release(self):
        commit = self.git("rev-parse", "HEAD")
        self.git("tag", "-a", "v0.17.2", "-m", "v0.17.2")
        data = self.archive("v0.17.2")
        staging = self.dir / "staging"
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
            if hasattr(tarfile, "data_filter"):  # Python 3.11.4 and later
                tar.extractall(staging, filter="data")
            else:
                tar.extractall(staging)
        metadata = self.dir / "metadata.json"
        metadata.write_text(json.dumps({"packages": [], "resolve": {"nodes": []}, "workspace_root": str(self.dir)}))
        done = subprocess.run(
            [sys.executable, "-I", str(staging / "cicada/scripts/cicada_source.py"), "--metadata", str(metadata)],
            capture_output=True,
            text=True,
            check=True,
        )
        report = json.loads(done.stdout)
        self.assertEqual(report["skill_version"], "0.17.2")
        self.assertEqual(report["skill_source"]["commit"], commit)

    def assert_refused(self, tag: str, words: str):
        done = self.package(tag)
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn(words, done.stderr)
        self.assertFalse((self.dir / "dist").exists() and any((self.dir / "dist").iterdir()), "nothing is written")

    def test_a_tag_that_differs_from_the_workspace_version_is_refused(self):
        self.git("tag", "v0.17.3")
        self.assert_refused("v0.17.3", "the workspace version is 0.17.2")

    def test_a_skill_of_another_version_is_refused(self):
        self.commit_version("0.17.3", skill="0.17.2")
        self.git("tag", "v0.17.3")
        self.assert_refused("v0.17.3", "SKILL.md says cicada-version 0.17.2")

    def test_a_missing_file_is_refused(self):
        self.commit_version("0.17.3", script=False)
        self.git("tag", "v0.17.3")
        self.assert_refused("v0.17.3", "skills/cicada/scripts/cicada_source.py is missing")

    def test_a_missing_or_malformed_tag_is_refused(self):
        self.assert_refused("v0.17.2", "there is no tag v0.17.2")
        self.assert_refused("main", "is not a version tag")

    def test_the_archive_bytes_are_those_of_the_skill_packager_before_the_shared_helpers(self):
        # A fully fixed repository: contents, names and dates, so its commit and archive never vary.
        fixed = self.dir / "fixed"
        (fixed / "skills" / "cicada" / "scripts").mkdir(parents=True)
        env = {"GIT_AUTHOR_DATE": "2026-01-01T00:00:00Z", "GIT_COMMITTER_DATE": "2026-01-01T00:00:00Z",
               "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1", "PATH": os.environ["PATH"]}
        run = lambda *args: subprocess.run([*GIT, *args], cwd=fixed, env=env, check=True, capture_output=True,
                                           text=True).stdout.strip()
        run("init", "-q")
        (fixed / "Cargo.toml").write_text(
            f'[workspace]\nmembers = []\n\n[workspace.package]\nversion = "1.2.3"\nrepository = "{REPOSITORY}"\n')
        (fixed / "skills/cicada/SKILL.md").write_text("---\nname: cicada\nmetadata:\n  cicada-version: 1.2.3\n---\n")
        (fixed / "skills/cicada/scripts/cicada_source.py").write_text("print('resolver')\n")
        run("add", "-A")
        run("commit", "-q", "-m", "fixed")
        run("tag", "v1.2.3")
        self.assertEqual(run("rev-parse", "HEAD"), "a90e84e1d7e95cacb0e562a791811303e09f694a")
        done = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "v1.2.3", str(self.dir / "out")], cwd=fixed,
                              capture_output=True, text=True, timeout=60)
        self.assertEqual(done.returncode, 0, done.stderr)
        data = (self.dir / "out" / "cicada-skill-v1.2.3.tar.gz").read_bytes()
        tar = gzip.decompress(data)
        # The tar stream the packager wrote before it shared its helpers with the fixture packager.
        self.assertEqual(hashlib.sha256(tar).hexdigest(), "a7aefd791a9020aeed575c387c396f8578ce44da892501a6fe32df9a7c27e7f6")
        # The same gzip framing: no name, time 0, level 9; compressing the stream again gives the same bytes.
        self.assertEqual(data[:10], bytes.fromhex("1f8b0800000000000" + "2ff"))
        again = io.BytesIO()
        with gzip.GzipFile(filename="", mode="wb", fileobj=again, mtime=0, compresslevel=9) as zipped:
            zipped.write(tar)
        self.assertEqual(again.getvalue(), data)

    def test_format_two_carries_both_skills_with_one_provenance(self):
        commit = self.commit_both("0.17.3")
        self.git("tag", "-a", "v0.17.3", "-m", "v0.17.3")
        data = self.archive("v0.17.3")
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
            members = tar.getmembers()
            self.assertEqual([m.name for m in members], DESIGN_NAMES + NAMES, "sorted, both roots, no marker")
            for member in members:
                self.assertEqual((member.mtime, member.uid, member.gid, member.uname, member.gname), (0, 0, 0, "", ""))
                self.assertTrue(member.isdir() or member.isfile(), member.name)
            read = lambda name: tar.extractfile(name).read()
            self.assertEqual(read("actor-design/SKILL.md"), (self.repo / "skills/actor-design/SKILL.md").read_bytes())
            self.assertEqual(read("actor-design/assets/design-template.md"),
                             (REPO / "skills/actor-design/assets/design-template.md").read_bytes())
            self.assertEqual(tar.getmember("actor-design/assets").mode, 0o755)
            self.assertEqual(tar.getmember("actor-design/SKILL.md").mode, 0o644)
            self.assertEqual(read("actor-design/SOURCE.json"), read("cicada/SOURCE.json"))
            source = json.loads(read("actor-design/SOURCE.json"))
        self.assertEqual(source, {"repository": REPOSITORY, "tag": "v0.17.3", "commit": commit, "version": "0.17.3"})

    def test_format_two_bytes_ignore_the_working_tree(self):
        self.commit_both("0.17.3")
        self.git("tag", "v0.17.3")
        first = self.archive("v0.17.3", "one")
        (self.repo / "skills/actor-design/SKILL.md").write_text("changed, not committed\n", encoding="utf-8")
        (self.repo / "skills/actor-design/assets/extra.md").write_text("untracked\n", encoding="utf-8")
        (self.repo / MARKER).unlink()
        self.assertEqual(self.archive("v0.17.3", "two"), first)

    def test_both_unpacked_skills_sit_side_by_side_and_the_sibling_resolver_runs(self):
        self.commit_both("0.17.3")
        self.git("tag", "v0.17.3")
        staging = self.dir / "staging"
        with tarfile.open(fileobj=io.BytesIO(self.archive("v0.17.3")), mode="r:gz") as tar:
            if hasattr(tarfile, "data_filter"):  # Python 3.11.4 and later
                tar.extractall(staging, filter="data")
            else:
                tar.extractall(staging)
        sibling = staging / "actor-design" / ".." / "cicada" / "scripts" / "cicada_source.py"
        metadata = self.dir / "metadata.json"
        metadata.write_text(json.dumps({"packages": [], "resolve": {"nodes": []}, "workspace_root": str(self.dir)}))
        done = subprocess.run([sys.executable, "-I", str(sibling), "--metadata", str(metadata)],
                              capture_output=True, text=True, check=True)
        report = json.loads(done.stdout)
        own = json.loads((staging / "actor-design" / "SOURCE.json").read_text())
        self.assertEqual((report["skill_source"]["commit"], report["skill_version"]), (own["commit"], own["version"]))

    def test_the_layout_follows_the_marker_and_the_design_tree(self):
        cases = {
            "design without marker": (dict(marker=None), "has skills/actor-design but no scripts/skill-archive-format"),
            "marker without newline": (dict(marker=b"2"), "must hold 2 and a newline"),
            "unknown format": (dict(marker=b"3\n"), "must hold 2 and a newline"),
            "spaces": (dict(marker=b" 2\n"), "must hold 2 and a newline"),
            "missing template": (dict(template=False), "skills/actor-design/assets/design-template.md is missing"),
        }
        for number, (name, (kwargs, words)) in enumerate(cases.items()):
            with self.subTest(name):
                version = f"0.18.{number}"
                self.commit_both(version, **kwargs)
                self.git("tag", f"v{version}")
                self.assert_refused(f"v{version}", words)

    def test_a_marker_without_the_design_skill_is_refused(self):
        (self.repo / "scripts").mkdir()
        (self.repo / MARKER).write_text("2\n")
        self.commit_version("0.17.3")
        self.git("tag", "v0.17.3")
        self.assert_refused("v0.17.3", "skills/actor-design/SKILL.md is missing")

    def test_a_design_skill_of_another_version_is_refused(self):
        self.commit_both("0.17.3", design="0.17.2")
        self.git("tag", "v0.17.3")
        self.assert_refused("v0.17.3", "skills/actor-design/SKILL.md says cicada-version 0.17.2")

    def test_a_linked_skill_file_or_marker_is_refused(self):
        self.write_design("0.17.3")
        template = self.repo / "skills/actor-design/assets/design-template.md"
        template.unlink()
        template.symlink_to("../SKILL.md")
        self.commit_version("0.17.3")
        self.git("tag", "v0.17.3")
        self.assert_refused("v0.17.3", "design-template.md at v0.17.3 is not a regular file (mode 120000)")
        self.write_design("0.17.4")
        marker = self.repo / MARKER
        (self.repo / "scripts" / "format").write_text("2\n")
        marker.unlink()
        marker.symlink_to("format")
        self.commit_version("0.17.4")
        self.git("tag", "v0.17.4")
        self.assert_refused("v0.17.4", "skill-archive-format at v0.17.4 is not a regular file (mode 120000)")

    def test_an_unsafe_or_unexpected_member_is_refused(self):
        module = load_module()
        members = {"cicada": (None, 0o755), "cicada/SKILL.md": (b"x", 0o644)}
        module.validate(module.build(members), members)
        both = dict(sorted({**members, "actor-design": (None, 0o755), "actor-design/SKILL.md": (b"y", 0o644)}.items()))
        module.validate(module.build(both), both)
        for name in ["../evil", "/etc/evil", "other/SKILL.md", "actor-design/../../evil", "cicada-fixtures/x"]:
            bad = {name: (b"x", 0o644)}
            with self.assertRaisesRegex(module.PackageError, "unsafe path"):
                module.validate(module.build(bad), bad)
        extra = dict(members, **{"cicada/extra.md": (b"y", 0o644)})
        with self.assertRaisesRegex(module.PackageError, "not"):
            module.validate(module.build(extra), members)
        # A repeated member, and a symbolic link where a file is expected.
        repeated = {"cicada": (None, 0o755), "cicada/SKILL.md": (b"x", 0o644)}
        with self.assertRaisesRegex(module.PackageError, "not"):
            module.validate(self.raw_tar([("cicada", None), ("cicada/SKILL.md", b"x"), ("cicada/SKILL.md", b"x")]),
                            repeated)
        with self.assertRaisesRegex(module.PackageError, "wrong type"):
            module.validate(self.raw_tar([("cicada", None), ("cicada/SKILL.md", "link")]), repeated)

    @staticmethod
    def raw_tar(entries: list[tuple[str, bytes | str | None]]) -> bytes:
        """A gzipped archive of these entries in order: None a folder, bytes a file, a string a link to that name."""
        tar = io.BytesIO()
        with tarfile.open(fileobj=tar, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for name, data in entries:
                info = tarfile.TarInfo(name)
                if data is None:
                    info.type, info.mode = tarfile.DIRTYPE, 0o755
                    archive.addfile(info)
                elif isinstance(data, str):
                    info.type, info.linkname, info.mode = tarfile.SYMTYPE, data, 0o644
                    archive.addfile(info)
                else:
                    info.size, info.mode = len(data), 0o644
                    archive.addfile(info, io.BytesIO(data))
        return gzip.compress(tar.getvalue())


if __name__ == "__main__":
    unittest.main()
