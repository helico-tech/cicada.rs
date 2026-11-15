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

    def test_an_unsafe_or_unexpected_member_is_refused(self):
        module = load_module()
        members = {"cicada": (None, 0o755), "cicada/SKILL.md": (b"x", 0o644)}
        module.validate(module.build(members), members)
        for name in ["../evil", "/etc/evil", "other/SKILL.md"]:
            bad = {name: (b"x", 0o644)}
            with self.assertRaisesRegex(module.PackageError, "unsafe path"):
                module.validate(module.build(bad), bad)
        extra = dict(members, **{"cicada/extra.md": (b"y", 0o644)})
        with self.assertRaisesRegex(module.PackageError, "not"):
            module.validate(module.build(extra), members)


if __name__ == "__main__":
    unittest.main()
