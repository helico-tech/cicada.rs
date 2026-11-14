"""Exercise ownership across real worktrees and preserve the review cursor on invalid acknowledgements."""

import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("docs-maintenance.py").resolve()


class OwnershipTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "docs"
        self.other = Path(self.temp.name) / "main"
        self.root.mkdir()
        self.git("init", "-b", "docs")
        self.git("config", "user.email", "docs@example.test")
        self.git("config", "user.name", "Docs test")
        (self.root / "scripts").mkdir()
        shutil.copyfile(SCRIPT, self.root / "scripts" / SCRIPT.name)
        (self.root / "docs").mkdir()
        (self.root / "docs" / "guide.md").write_text("# Guide\n")
        (self.root / "README.md").write_text('Use tag = "v0.17.1"\n')
        self.git("add", ".")
        self.git("commit", "-m", "Initial")
        self.git("branch", "main")
        self.git("worktree", "add", str(self.other), "main")
        self.run_script("claim")
        common = self.git("rev-parse", "--git-common-dir").stdout.strip()
        self.state = (self.root / common / "cicada-docs" / "owner.json").resolve()

    def git(self, *args, cwd=None, check=True):
        return subprocess.run(["git", *args], cwd=cwd or self.root, text=True, capture_output=True, check=check)

    def run_script(self, *args, cwd=None, check=True):
        return subprocess.run(
            ["python3", str(SCRIPT), *args], cwd=cwd or self.root, text=True, capture_output=True, check=check
        )

    def test_other_worktree_cannot_commit_prose(self):
        (self.other / "docs" / "guide.md").write_text("Competing prose\n")
        self.git("add", "docs/guide.md", cwd=self.other)
        result = self.git("commit", "-m", "Competing writer", cwd=self.other, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("belongs to the docs session", result.stderr)

    def test_owner_commit_and_main_integration_are_allowed(self):
        (self.root / "docs" / "guide.md").write_text("# Better guide\n")
        self.git("add", "docs/guide.md")
        self.git("commit", "-m", "Docs owner update")
        self.git("merge", "--no-ff", "docs", "-m", "Integrate docs", cwd=self.other)
        self.assertEqual((self.other / "docs" / "guide.md").read_text(), "# Better guide\n")

    def test_release_pins_and_generated_artifacts_are_allowed(self):
        (self.other / "README.md").write_text('Use tag = "v0.17.2"\n')
        (self.other / "docs" / "openapi.json").write_text('{}\n')
        self.git("add", "README.md", "docs/openapi.json", cwd=self.other)
        self.git("commit", "-m", "Release metadata", cwd=self.other)

    def test_rename_out_of_protected_docs_is_blocked(self):
        self.git("mv", "docs/guide.md", "guide.md", cwd=self.other)
        result = self.run_script("check-owner", cwd=self.other, check=False)
        self.assertNotEqual(result.returncode, 0)

    def test_cannot_acknowledge_unmerged_docs(self):
        cursor = json.loads(self.state.read_text())["reviewed_main"]
        (self.root / "docs" / "guide.md").write_text("New docs\n")
        self.git("add", "docs/guide.md")
        self.git("commit", "-m", "Unmerged docs")
        result = self.run_script("acknowledge", "HEAD", check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(json.loads(self.state.read_text())["reviewed_main"], cursor)

    def test_scan_reports_local_main_changes_and_release_removes_hooks(self):
        (self.other / "source.rs").write_text("fn new_api() {}\n")
        self.git("add", "source.rs", cwd=self.other)
        self.git("commit", "-m", "New API", cwd=self.other)
        scan = json.loads(self.run_script("scan").stdout)
        self.assertIn("source.rs", scan["refs"]["main"]["files"])
        self.run_script("release")
        self.assertFalse(self.state.exists())
        self.assertFalse((self.root / ".git" / "hooks" / "pre-commit").exists())


if __name__ == "__main__":
    unittest.main()
