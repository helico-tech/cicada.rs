"""Tests of scripts/package-fixtures.py (ADR 0047). Each test packages a tag of a temporary Git repository that holds
the fixtures of this checkout; none packages a real Cicada tag, publishes anything or moves a tag."""

from __future__ import annotations

import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parent
SCRIPT = SCRIPTS / "package-fixtures.py"
GIT = ["git", "-c", "user.name=test", "-c", "user.email=test@example.test", "-c", "commit.gpgsign=false",
       "-c", "tag.gpgsign=false"]
REPOSITORY = "https://example.test/cicada.rs"
FIXTURES = ["canonical.json", "contracts.json", "leases.json", "status.json"]
NAMES = ["cicada-fixtures", "cicada-fixtures/SOURCE.json", *(f"cicada-fixtures/{name}" for name in FIXTURES)]


class Packaging(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.repo = self.dir / "repo"
        (self.repo / "fixtures").mkdir(parents=True)
        self.git("init", "-q")
        for name in FIXTURES:
            shutil.copy(REPO / "fixtures" / name, self.repo / "fixtures" / name)
        self.commit_version("0.18.12")

    def git(self, *args: str) -> str:
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        return subprocess.run([*GIT, *args], cwd=self.repo, env=env, check=True, capture_output=True,
                              text=True).stdout.strip()

    def commit_version(self, version: str) -> str:
        (self.repo / "Cargo.toml").write_text(
            f'[workspace]\nmembers = []\n\n[workspace.package]\nversion = "{version}"\nrepository = "{REPOSITORY}"\n')
        self.git("add", "-A")
        self.git("commit", "-q", "--allow-empty", "-m", f"version {version}")
        return self.git("rev-parse", "HEAD")

    def package(self, tag: str, output: str = "dist") -> subprocess.CompletedProcess:
        return subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), tag, str(self.dir / output)], cwd=self.repo,
                              capture_output=True, text=True, timeout=60)

    def archive(self, tag: str, output: str = "dist") -> bytes:
        done = self.package(tag, output)
        self.assertEqual(done.returncode, 0, done.stderr)
        path = self.dir / output / f"cicada-fixtures-{tag}.tar.gz"
        self.assertEqual(done.stdout.strip(), str(path))
        return path.read_bytes()

    def test_an_annotated_tag_packages_the_four_fixtures_of_its_commit(self):
        commit = self.git("rev-parse", "HEAD")
        self.git("tag", "-a", "v0.18.12", "-m", "v0.18.12")
        data = self.archive("v0.18.12")
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
            members = tar.getmembers()
            self.assertEqual([m.name for m in members], NAMES)
            for member in members:
                self.assertEqual((member.mtime, member.uid, member.gid, member.uname, member.gname), (0, 0, 0, "", ""))
                self.assertEqual((member.isdir(), member.mode), (True, 0o755) if member.name == NAMES[0]
                                 else (False, 0o644))
            for name in FIXTURES:
                self.assertEqual(tar.extractfile(f"cicada-fixtures/{name}").read(),
                                 (REPO / "fixtures" / name).read_bytes())
            source = json.loads(tar.extractfile("cicada-fixtures/SOURCE.json").read())
        self.assertEqual(source, {"repository": REPOSITORY, "tag": "v0.18.12", "commit": commit, "version": "0.18.12"})
        self.assertNotIn(str(self.dir).encode(), data)

    def test_packaging_again_gives_the_same_bytes_whatever_the_working_tree_holds(self):
        self.git("tag", "v0.18.12")
        first = self.archive("v0.18.12", "one")
        (self.repo / "fixtures" / "status.json").write_text('{"format": 2}\n')
        (self.repo / "fixtures" / "extra.json").write_text('{"format": 1}\n')
        (self.repo / "Cargo.toml").write_text('[workspace.package]\nversion = "9.9.9"\n')
        self.assertEqual(self.archive("v0.18.12", "two"), first)
        self.assertEqual(self.archive("v0.18.12", "two"), first)

    def test_neither_packager_writes_bytecode_next_to_itself_without_b(self):
        # `-I` ignores PYTHONDONTWRITEBYTECODE; loading the shared helpers must still leave the checkout clean.
        copies = self.dir / "copies"
        copies.mkdir()
        for name in ("package-fixtures.py", "package-skill.py", "release_archive.py"):
            shutil.copy(SCRIPTS / name, copies / name)
        self.git("tag", "v0.18.12")
        for packager in ("package-fixtures.py", "package-skill.py"):
            subprocess.run([sys.executable, "-I", str(copies / packager), "v0.18.12", str(self.dir / "out")],
                           cwd=self.repo, capture_output=True, timeout=60)
        self.assertEqual(sorted(p.name for p in copies.iterdir()),
                         ["package-fixtures.py", "package-skill.py", "release_archive.py"])

    def assert_refused(self, tag: str, words: str):
        done = self.package(tag)
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn(words, done.stderr)
        self.assertFalse((self.dir / "dist").exists() and any((self.dir / "dist").iterdir()), "nothing is written")

    def refused_after(self, change, words: str):
        change()
        self.commit_version("0.18.13")
        self.git("tag", "v0.18.13")
        self.assert_refused("v0.18.13", words)

    def test_a_tag_or_version_that_does_not_match_is_refused(self):
        self.git("tag", "v0.18.13")
        self.assert_refused("v0.18.13", "the workspace version is 0.18.12")
        self.assert_refused("v0.18.14", "there is no tag v0.18.14")
        self.assert_refused("main", "is not a version tag")

    def test_a_missing_file_is_refused(self):
        self.refused_after(lambda: (self.repo / "fixtures" / "leases.json").unlink(), "not ['canonical.json'")

    def test_an_extra_file_is_refused(self):
        self.refused_after(lambda: (self.repo / "fixtures" / "more.json").write_text('{"format": 1}\n'), "more.json")

    def test_a_folder_inside_is_refused(self):
        def change():
            (self.repo / "fixtures" / "status.json").unlink()
            (self.repo / "fixtures" / "status.json").mkdir()
            (self.repo / "fixtures" / "status.json" / "x.json").write_text("{}\n")
        self.refused_after(change, "fixtures/status.json at v0.18.13 is not a regular file (mode 040000)")

    def test_a_link_is_refused(self):
        def change():
            (self.repo / "fixtures" / "status.json").unlink()
            os.symlink("leases.json", self.repo / "fixtures" / "status.json")
        self.refused_after(change, "is not a regular file (mode 120000)")

    def test_missing_fixtures_or_a_file_in_their_place_is_refused(self):
        self.refused_after(lambda: self.git("rm", "-rq", "fixtures"), "fixtures/ is missing at v0.18.13")
        (self.repo / "fixtures").write_text("not a folder\n")
        self.commit_version("0.18.14")
        self.git("tag", "v0.18.14")
        self.assert_refused("v0.18.14", "fixtures at v0.18.14 is not a folder")

    def test_a_file_that_is_no_format_1_fixture_is_refused(self):
        wrong = "is not a fixture of format 1"
        for text, words in [("not json", "is not JSON"), ('{"format": 2}', wrong), ("[1]", wrong),
                            ('{"format": "1"}', wrong), ('{"format": true}', wrong), ('{"format": 1.0}', wrong)]:
            with self.subTest(text):
                (self.repo / "fixtures" / "contracts.json").write_text(text)
                self.commit_version("0.18.12")
                self.git("tag", "-f", "v0.18.12")
                self.assert_refused("v0.18.12", f"fixtures/contracts.json at v0.18.12 {words}")


if __name__ == "__main__":
    unittest.main()
