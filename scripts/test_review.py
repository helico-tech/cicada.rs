"""Tests of `review.py take`, by id and without one, in a temporary repository."""

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
        self.assertIn(f"{second}  taken  tester", listing)

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


if __name__ == "__main__":
    unittest.main()
