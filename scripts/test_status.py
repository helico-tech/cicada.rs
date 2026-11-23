"""Tests of the `release:` line of `status.py`, in a temporary repository."""

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


if __name__ == "__main__":
    unittest.main()
