"""Tests of `review.py`: `take` by id and without one, and code reviews by another model family, in a temporary
repository."""

import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parent / "review.py"


class TakeTest(unittest.TestCase):
    def setUp(self):
        self.root = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / "scripts").mkdir()
        shutil.copy(SCRIPT, self.root / "scripts" / "review.py")
        for change in ("one", "two"):
            (self.root / "openspec" / "changes" / change).mkdir(parents=True)
        subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)

    def review(self, *args):
        return subprocess.run(
            [sys.executable, str(self.root / "scripts" / "review.py"), *args], capture_output=True, text=True
        )

    def request(self, change):
        result = self.review("request", "design", change)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_take_by_id_claims_that_request(self):
        first, second = self.request("one"), self.request("two")
        result = self.review("take", second, "--by", "tester")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"Took {second}", result.stdout)
        listing = self.review("list").stdout
        self.assertIn(f"{first}  open", listing)
        self.assertIn(f"{second}  taken  taken by tester", listing)

    def test_markers_are_plain_files(self):
        review = self.request("one")
        self.assertEqual(self.review("take", review, "--by", "tester").returncode, 0)
        markers = sorted((self.root / ".reviews").iterdir())
        self.assertIn(self.root / ".reviews" / f"{review}.taken", markers)
        for marker in markers:
            with self.subTest(marker.name):
                self.assertEqual(marker.stat().st_mode & 0o111, 0)

    def test_take_of_a_taken_id_fails(self):
        review = self.request("one")
        self.assertEqual(self.review("take", review).returncode, 0)
        result = self.review("take", review)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"review: {review} is taken, not open", result.stderr)

    def test_take_of_a_done_id_fails(self):
        review = self.request("one")
        self.assertEqual(self.review("take", review).returncode, 0)
        report = self.root / "report.md"
        report.write_text("Verdict: pass\n")
        self.assertEqual(self.review("done", review, str(report)).returncode, 0)
        result = self.review("take", review)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"review: {review} is done, not open", result.stderr)

    def test_take_of_a_missing_id_fails(self):
        self.request("one")
        result = self.review("take", "two.code.1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("review: no request two.code.1", result.stderr)

    def test_take_without_an_id_takes_the_oldest_open_request(self):
        first, second = self.request("one"), self.request("two")
        result = self.review("take")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"Took {first}", result.stdout)
        result = self.review("take")
        self.assertIn(f"Took {second}", result.stdout)
        result = self.review("take")
        self.assertEqual(result.returncode, 1)
        self.assertIn("review: no open request", result.stdout)


class FamilyTest(unittest.TestCase):
    """A code review is taken by a model family other than its builder's (ADR 0073)."""

    def setUp(self):
        self.root = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / "scripts").mkdir()
        shutil.copy(SCRIPT, self.root / "scripts" / "review.py")
        for change in ("one", "two"):
            (self.root / "openspec" / "changes" / change).mkdir(parents=True)
        git = ["git", "-c", "user.name=test", "-c", "user.email=test@example.com", "-c", "commit.gpgSign=false"]
        subprocess.run([*git, "init", "-q", "-b", "main"], cwd=self.root, check=True)
        subprocess.run([*git, "add", "-A"], cwd=self.root, check=True)
        subprocess.run([*git, "commit", "-q", "-m", "First"], cwd=self.root, check=True)

    def review(self, *args):
        return subprocess.run(
            [sys.executable, str(self.root / "scripts" / "review.py"), *args], capture_output=True, text=True
        )

    def request(self, change, built_by):
        result = self.review("request", "code", change, "--head", "HEAD", "--built-by", built_by)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_the_builder_is_kept_and_listed(self):
        review = self.request("one", "claude-opus-5-5")
        self.assertIn("built-by: claude-opus-5-5\n", (self.root / ".reviews" / f"{review}.md").read_text())
        self.assertEqual(self.review("take", review, "--by", "gpt-6.2-sol").returncode, 0)
        listing = self.review("list").stdout
        self.assertIn(f"{review}  taken  built by claude-opus-5-5  taken by gpt-6.2-sol", listing)

    def test_take_by_id_refuses_the_builders_family(self):
        review = self.request("one", "claude-opus-5-5")
        result = self.review("take", review, "--by", "claude-reviewer")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"review: {review}: built by claude-opus-5-5; a reviewer of another family", result.stderr)
        self.assertFalse((self.root / ".reviews" / f"{review}.taken").exists())
        self.assertEqual(self.review("take", review, "--by", "codex-gpt-6.2").returncode, 0)

    def test_gpt_builds_are_refused_to_codex_and_openai_names(self):
        review = self.request("one", "gpt-6.2-sol")
        for name in ("codex", "openai-reviewer", "GPT-6.2"):
            with self.subTest(name):
                self.assertNotEqual(self.review("take", review, "--by", name).returncode, 0)
        self.assertEqual(self.review("take", review, "--by", "Claude-opus").returncode, 0)

    def test_take_without_an_id_skips_the_builders_family(self):
        first = self.request("one", "claude-opus-5-5")
        second = self.request("two", "gpt-6.2-sol")
        result = self.review("take", "--by", "claude-reviewer")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"Took {second}", result.stdout)
        result = self.review("take", "--by", "claude-reviewer")
        self.assertEqual(result.returncode, 1)
        self.assertIn("review: no open request", result.stdout)
        self.assertIn(f"{first}  open", self.review("list").stdout)

    def test_a_same_family_review_is_recorded(self):
        review = self.request("one", "claude-opus-5-5")
        result = self.review("take", review, "--by", "claude-reviewer", "--same-family", "Codex unavailable")
        self.assertEqual(result.returncode, 0, result.stderr)
        report = self.root / "report.md"
        report.write_text("Verdict: pass\n\nFine.\n")
        self.assertEqual(self.review("done", review, str(report)).returncode, 0)
        result_text = (self.root / ".reviews" / f"{review}.result.md").read_text()
        self.assertTrue(result_text.startswith("Verdict: pass\n"))
        self.assertIn("Same-family review by claude-reviewer: Codex unavailable", result_text)
        self.assertIn("same family: Codex unavailable  Verdict: pass", self.review("list").stdout)

    def test_same_family_is_not_recorded_for_another_family(self):
        review = self.request("one", "claude-opus-5-5")
        self.assertEqual(self.review("take", review, "--by", "gpt-6.2-sol", "--same-family", "why").returncode, 0)
        self.assertNotIn("same-family", (self.root / ".reviews" / f"{review}.taken").read_text())

    def test_names_without_a_family_are_refused_for_code_reviews(self):
        result = self.review("request", "code", "one", "--head", "HEAD")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("needs --built-by", result.stderr)
        result = self.review("request", "code", "one", "--head", "HEAD", "--built-by", "someone")
        self.assertNotEqual(result.returncode, 0)
        review = self.request("one", "claude-opus-5-5")
        for by in ([], ["--by", "unknown"]):
            with self.subTest(by):
                result = self.review("take", review, *by)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("a code review needs --by", result.stderr)

    def test_names_with_a_line_break_are_refused(self):
        result = self.review("request", "code", "one", "--head", "HEAD", "--built-by", "claude-x\nbuilt-by: gpt")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("needs --built-by", result.stderr)
        review = self.request("one", "claude-opus-5-5")
        result = self.review("take", review, "--by", "gpt-x\nsame-family: yes")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--by needs a name on one line", result.stderr)

    def test_a_design_review_refuses_a_name_with_a_line_break(self):
        result = self.review("request", "design", "one")
        self.assertEqual(result.returncode, 0, result.stderr)
        review = result.stdout.strip()
        result = self.review("take", review, "--by", "claude-x\nsame-family: yes")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--by needs a name on one line", result.stderr)
        self.assertFalse((self.root / ".reviews" / f"{review}.taken").exists())

    def test_a_same_family_review_needs_a_reason_on_one_line(self):
        review = self.request("one", "claude-opus-5-5")
        for reason in ("", "   ", "two\nlines"):
            with self.subTest(reason=reason):
                result = self.review("take", review, "--by", "claude-reviewer", "--same-family", reason)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("--same-family needs a reason on one line", result.stderr)
        self.assertFalse((self.root / ".reviews" / f"{review}.taken").exists())

    def test_design_reviews_take_any_reviewer(self):
        result = self.review("request", "design", "one")
        review = result.stdout.strip()
        self.assertEqual(self.review("take", review).returncode, 0)
        self.assertIn("by: unknown\n", (self.root / ".reviews" / f"{review}.taken").read_text())


if __name__ == "__main__":
    unittest.main()
