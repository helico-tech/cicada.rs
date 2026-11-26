"""Tests of the `release:`, `build:` and `problems:` lines of `status.py`, in a temporary repository."""

import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parent / "status.py"
GIT = ["git", "-c", "user.name=test", "-c", "user.email=test@example.com", "-c", "commit.gpgSign=false",
       "-c", "tag.gpgSign=false"]
RELEASED = "## 0.1.0 — 2026-11-01\n\n- First.\n"
UNRELEASED = "## 0.1.1 — unreleased\n\n- Next.\n\n"


class ReleaseLineTest(unittest.TestCase):
    def setUp(self):
        self.root = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / "scripts").mkdir()
        shutil.copy(SCRIPT, self.root / "scripts" / "status.py")
        self.changelog(RELEASED)
        self.archive("2026-11-01-first", "")
        self.git("init", "-q")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "First")
        self.git("tag", "v0.1.0")

    def git(self, *args):
        subprocess.run([*GIT, *args], cwd=self.root, check=True, capture_output=True)

    def changelog(self, sections):
        (self.root / "CHANGELOG.md").write_text("# Changelog\n\nReleases.\n\n" + sections)

    def archive(self, name, lane, version="0.1.1"):
        folder = self.root / "openspec" / "changes" / "archive" / name
        folder.mkdir(parents=True)
        (folder / "proposal.md").write_text(f"# Proposal\n\n## Why\n\nBecause.\n{lane}\n## Order\n\nAfter `x`.\n")
        (folder / "docs-handoff.md").write_text(f"# Docs handoff for {name}\n\nVersion: {version}. Details.\n")

    def ready(self, name):
        folder = self.root / "openspec" / "changes" / name
        folder.mkdir(parents=True)
        (folder / "proposal.md").write_text("# Proposal\n\n## Order\n\nAfter `first`.\n")
        (folder / "tasks.md").write_text("- [ ] 1. A task.\n")

    def land(self, name, lane="", version="0.1.1"):
        self.archive(name, lane, version)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", f"Archive {name}")

    def release_line(self):
        result = subprocess.run(
            [sys.executable, str(self.root / "scripts" / "status.py")], capture_output=True, text=True
        )
        lines = [line for line in result.stdout.splitlines() if line.startswith("release:")]
        self.assertEqual(len(lines), 1, result.stdout + result.stderr)
        return lines[0]

    def test_nothing_unreleased(self):
        self.land("2026-11-02-process", "\n## Lane\n\nLane: fast\n")
        self.assertEqual(self.release_line(), "release:  nothing unreleased")

    def test_an_unreleased_batch_waits_for_a_ready_change(self):
        self.changelog(UNRELEASED + RELEASED)
        self.land("2026-11-02-behaviour")
        self.ready("next")
        self.assertEqual(self.release_line(), "release:  0.1.1 unreleased, 1 change landed; wait: next is ready")

    def test_a_batch_of_fast_lane_changes_only_waits(self):
        self.changelog(UNRELEASED + RELEASED)
        self.land("2026-11-02-rustdoc", "\n## Lane\n\nLane: fast\n")
        self.land("2026-11-03-tests", "\n## Lane\n\n`Lane: fast`\n")
        self.assertEqual(
            self.release_line(),
            "release:  0.1.1 unreleased, 2 changes landed; wait: no landed change outside the fast lane",
        )

    def test_a_batch_with_a_normal_change_is_released_when_nothing_is_ready(self):
        self.changelog(UNRELEASED + RELEASED)
        self.land("2026-11-02-rustdoc", "\n## Lane\n\nLane: fast\n")
        self.land("2026-11-03-behaviour")
        self.assertEqual(
            self.release_line(),
            "release:  0.1.1 unreleased, 2 changes landed; release now: no change is ready or in build",
        )

    def test_an_unreleased_heading_without_a_landed_change_waits(self):
        self.changelog(UNRELEASED + RELEASED)
        self.assertEqual(self.release_line(), "release:  0.1.1 unreleased, 0 changes landed; wait: no change landed")

    def test_a_change_without_a_version_is_not_part_of_the_batch(self):
        self.changelog(UNRELEASED + RELEASED)
        self.land("2026-11-02-process", version="none")
        self.land("2026-11-03-rustdoc", "\n## Lane\n\nLane: fast\n")
        self.assertEqual(
            self.release_line(),
            "release:  0.1.1 unreleased, 1 change landed; wait: no landed change outside the fast lane",
        )

    def test_release_now_releases_at_once(self):
        self.changelog(UNRELEASED + RELEASED)
        self.land("2026-11-02-security-fix", "\n## Lane\n\nLane: fast\nRelease: now\n")
        self.ready("next")
        self.assertEqual(
            self.release_line(),
            "release:  0.1.1 unreleased, 1 change landed; release now: security-fix says Release: now",
        )


class BuildLineTest(unittest.TestCase):
    """Two builders when their changes share no crate (ADR 0073)."""

    def setUp(self):
        self.root = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / "scripts").mkdir()
        shutil.copy(SCRIPT, self.root / "scripts" / "status.py")
        (self.root / "openspec" / "changes" / "archive").mkdir(parents=True)
        subprocess.run([*GIT, "init", "-q"], cwd=self.root, check=True, capture_output=True)

    def change(self, name, after, crates=None, tasks="- [ ] 1. A task.\n", folder=None):
        folder = folder or self.root / "openspec" / "changes" / name
        folder.mkdir(parents=True)
        impact = f"## Impact\n\nCrates: {crates}\n\nMore.\n" if crates is not None else "## Impact\n\nSome crates.\n"
        after = f"After `{after}`." if after else "First."
        (folder / "proposal.md").write_text(f"# Proposal\n\n## Order\n\n{after}\n\n{impact}")
        if tasks is not None:
            (folder / "tasks.md").write_text(tasks)

    def building(self, name, after, crates=None):
        self.change(name, after, crates, tasks="- [x] 1. Done.\n- [ ] 2. Open.\n")

    def archived(self, name, after):
        self.change(name, after, "none", folder=self.root / "openspec" / "changes" / "archive" / f"2026-11-20-{name}")

    def status(self):
        result = subprocess.run(
            [sys.executable, str(self.root / "scripts" / "status.py")], capture_output=True, text=True
        )
        return result

    def build_line(self):
        lines = [line for line in self.status().stdout.splitlines() if line.startswith("build:")]
        self.assertEqual(len(lines), 1)
        return lines[0]

    def names(self):
        line = self.build_line()
        return [part.split(" (")[0] for part in line[len("build:    ") :].split(" and ")]

    def test_disjoint_crates_build_together(self):
        self.building("placement", None, "cicada-actors")
        self.change("tray", "placement", "cicada-tray")
        self.assertEqual(self.names(), ["placement", "tray"])

    def test_a_shared_crate_is_passed_over_by_a_later_disjoint_change(self):
        self.building("placement", None, "`cicada-actors`, `cicada-containers`.")
        self.change("settings", "placement", "cicada-actors")
        self.change("process", "settings", "none")
        self.assertEqual(self.names(), ["placement", "process"])

    def test_a_change_without_a_crates_line_is_built_only_alone(self):
        self.building("placement", None, "cicada-actors")
        self.change("unknown", "placement")
        self.assertEqual(self.names(), ["placement"])

    def test_a_change_without_a_crates_line_goes_alone_when_nothing_is_in_build(self):
        self.change("unknown", None)
        self.change("tray", "unknown", "cicada-tray")
        self.assertEqual(self.names(), ["unknown"])

    def test_a_change_in_build_without_a_crates_line_takes_both_slots(self):
        self.building("unknown", None)
        self.change("process", "unknown", "none")
        self.assertEqual(self.names(), ["unknown"])

    def test_crates_none_pairs_with_anything(self):
        self.building("process", None, "none")
        self.change("placement", "process", "cicada-actors and cicada-app")
        self.change("tray", "placement", "cicada-tray")
        self.assertEqual(self.names(), ["process", "placement"])

    def test_two_ready_changes_fill_both_slots(self):
        self.change("placement", None, "cicada-actors")
        self.change("tray", "placement", "cicada-tray")
        self.change("process", "tray", "none")
        self.assertEqual(self.names(), ["placement", "tray"])

    def test_nothing_ready(self):
        self.change("idea", None, "none", tasks=None)
        self.assertEqual(self.build_line(), "build:    nothing; no open change is ready")

    def test_two_changes_in_build_sharing_a_crate_are_a_problem(self):
        self.building("placement", None, "cicada-actors, cicada-app")
        self.building("settings", "placement", "cicada-actors")
        result = self.status()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("  - placement and settings are both in build and share cicada-actors", result.stdout)

    def test_three_changes_in_build_are_a_problem_and_the_line_names_two(self):
        self.building("placement", None, "cicada-actors")
        self.building("tray", "placement", "cicada-tray")
        self.building("process", "tray", "none")
        result = self.status()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("3 changes hold a builder (placement, tray, process); at most 2 do", result.stdout)
        self.assertEqual(self.names(), ["placement", "tray"])

    def test_an_order_naming_an_archived_change_resolves_through_it(self):
        # first <- second <- third; second landed before first.
        self.building("first", "landed", "cicada-actors")
        self.archived("second", "first")
        self.change("third", "second", "cicada-tray")
        (self.root / "openspec" / "changes" / "first" / "proposal.md").write_text(
            "# Proposal\n\n## Order\n\nAfter `landed`, before `second`.\n\n## Impact\n\nCrates: cicada-actors\n"
        )
        result = self.status()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("problems: none", result.stdout)
        self.assertEqual(self.names(), ["first", "third"])
        rows = [line.split()[1] for line in result.stdout.splitlines()[1:3]]
        self.assertEqual(rows, ["first", "third"])


if __name__ == "__main__":
    unittest.main()
