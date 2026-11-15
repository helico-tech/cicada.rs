"""The pinned release of scripts/gate.py against a temporary bare origin (reached through url.insteadOf, so origin
still names a GitHub repository) and a fake gh. No real push, tag or GitHub Release is made."""

import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from gate_testing import Sandbox, copy_script, flag_command, install_fake_gh, policy  # noqa: E402

GITHUB = "https://github.com/acme/cicada.git"
FILES = {
    "Cargo.toml": '[workspace]\nmembers = ["c"]\nresolver = "2"\n\n[workspace.package]\nversion = "0.9.0"\n'
                  'repository = "https://github.com/acme/cicada"\n',
    "c/Cargo.toml": '[package]\nname = "c"\nversion.workspace = true\nedition = "2021"\n',
    "c/src/lib.rs": "",
    "skills/cicada/SKILL.md": "---\nname: cicada\nmetadata:\n  cicada-version: 0.9.0\n---\n# Skill\n",
    "skills/cicada/scripts/cicada_source.py": "print('resolver')\n",
    "CHANGELOG.md": "# Changelog\n\n## 0.9.0 — 2026-11-01\n\n- Old.\n",
}
RELEASE = {
    "Cargo.toml": FILES["Cargo.toml"].replace("0.9.0", "1.0.0"),
    "skills/cicada/SKILL.md": FILES["skills/cicada/SKILL.md"].replace("0.9.0", "1.0.0"),
    "CHANGELOG.md": "# Changelog\n\n## 1.0.0 — 2026-11-15\n\n- First.\n\n## 0.9.0 — 2026-11-01\n\n- Old.\n",
}
NOTES = "\n- First.\n\n"


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.s = Sandbox(self, files=FILES)
        copy_script(self.s, "release-check.sh")
        copy_script(self.s, "package-skill.py")
        self.s.commit("Release scripts")
        self.bare = self.s.dir / "origin.git"
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(self.bare)], env=self.s.env, check=True)
        self.s.git("remote", "add", "origin", GITHUB)
        self.s.git("config", f"url.{self.bare}.insteadOf", GITHUB)
        self.s.git("push", "-q", "origin", "main")
        self.s.git("fetch", "-q", "origin")
        self.assertEqual(self.s.gate("install-hook").returncode, 0)
        self.gh_state = install_fake_gh(self.s, self.bare)
        self.c = self.s.commit("Release 1.0.0", RELEASE)

    def gh(self) -> dict:
        return json.loads(self.gh_state.read_text())

    def set_gh(self, **changes):
        state = self.gh()
        state.update(changes)
        self.gh_state.write_text(json.dumps(state))

    def remote(self, ref):
        done = subprocess.run(["git", "--git-dir", str(self.bare), "rev-parse", "--verify", "-q", ref],
                              capture_output=True, text=True, env=self.s.env)
        return done.stdout.strip() or None

    def release(self, *args):
        return self.s.gate("release", "v1.0.0", *args)

    def state(self) -> dict:
        return json.loads((self.s.state / "release" / "v1.0.0.json").read_text())

    def writes(self) -> list:
        return [c for c in self.gh()["calls"] if c[:2] in (["release", "create"], ["release", "upload"])
                or c[:3] == ["api", "-X", "PATCH"]]

    def assert_published(self):
        (found,) = self.gh()["releases"]
        self.assertFalse(found["draft"])
        self.assertEqual((found["tag_name"], found["name"], found["body"]), ("v1.0.0", "v1.0.0", NOTES))
        (asset,) = found["assets"]
        self.assertEqual(asset["name"], "cicada-skill-v1.0.0.tar.gz")
        with tarfile.open(fileobj=io.BytesIO(Path(asset["file"]).read_bytes()), mode="r:gz") as archive:
            source = json.loads(archive.extractfile("cicada/SOURCE.json").read())
        self.assertEqual((source["commit"], source["tag"]), (self.c, "v1.0.0"))
        self.assertEqual(self.remote("refs/heads/main"), self.c)
        tag = self.s.git("rev-parse", "refs/tags/v1.0.0")
        self.assertEqual(self.s.git("cat-file", "-t", tag), "tag", "an annotated tag")
        self.assertEqual(self.remote("refs/tags/v1.0.0"), tag, "the same raw tag object")
        self.assertEqual(self.state()["stage"], "published")

    def test_a_release_checks_publishes_and_leaves_the_caller_alone(self):
        self.s.write("README.md", "# Dirty caller file\n")
        self.s.write("scratch.txt", "untracked\n")
        branch = self.s.git("symbolic-ref", "HEAD")
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assert_published()
        self.assertEqual(self.s.record("full")["status"], "pass")
        self.assertEqual(self.s.record("e2e")["status"], "pass")
        self.assertEqual((self.s.root / "README.md").read_text(), "# Dirty caller file\n")
        self.assertTrue((self.s.root / "scratch.txt").exists())
        self.assertEqual(self.s.git("symbolic-ref", "HEAD"), branch)
        self.assertEqual(self.s.git("rev-parse", "HEAD"), self.c)
        self.assertEqual((self.s.state / "release" / "v1.0.0" / "notes.md").read_text(), NOTES)
        # Running it again leaves the published release untouched.
        writes = len(self.writes())
        again = self.release()
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertIn("already published", again.stderr)
        self.assertEqual(len(self.writes()), writes)

    def test_a_retry_after_a_github_failure_resumes_the_pinned_commit(self):
        self.set_gh(fail=["create"])
        failed = self.release()
        self.assertEqual(failed.returncode, 1)
        self.assertEqual(self.state()["stage"], "tag-pushed")
        self.assertEqual(self.remote("refs/heads/main"), self.c, "main stays pushed")
        tag = self.remote("refs/tags/v1.0.0")
        self.assertIsNotNone(tag)
        moved = self.s.commit("Caller moves on", {"later.rs": "\n"})
        refused = self.release(moved)
        self.assertIn("cannot release another commit", refused.stderr)
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assert_published()
        self.assertEqual(self.remote("refs/tags/v1.0.0"), tag)

    def test_uncertain_api_results_are_reinspected(self):
        self.set_gh(fail=["create-after", "publish"])
        failed = self.release()
        self.assertEqual(failed.returncode, 1, "the draft exists but publishing failed")
        self.assertTrue(self.gh()["releases"][0]["draft"])
        self.assertEqual(self.state()["stage"], "draft-verified")
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assert_published()
        creates = [c for c in self.writes() if c[:2] == ["release", "create"]]
        self.assertEqual(len(creates), 1, "the existing draft was resumed, not created again")

    def draft(self, body=NOTES, assets=None, tag="v1.0.0"):
        self.set_gh(releases=[{"id": 7, "tag_name": tag, "name": tag, "draft": True, "body": body,
                               "assets": assets or []}])

    def test_a_matching_draft_without_its_asset_gets_it_uploaded(self):
        self.draft()
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assert_published()
        self.assertTrue(any(c[:2] == ["release", "upload"] for c in self.writes()))

    def test_a_draft_asset_without_digest_is_downloaded_and_compared(self):
        self.set_gh(digests=False, fail=["publish"])
        self.assertEqual(self.release().returncode, 1)
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(any("/releases/assets/" in c[1] for c in self.gh()["calls"] if c[0] == "api"))

    def test_conflicting_drafts_are_never_clobbered(self):
        other = self.s.dir / "other.tar.gz"
        other.write_bytes(b"someone else's")
        cases = {
            "notes": dict(body="Other notes\n"),
            "extra asset": dict(assets=[{"id": 9, "name": "extra.zip", "digest": "sha256:00", "file": str(other)}]),
            "other hash": dict(assets=[{"id": 9, "name": "cicada-skill-v1.0.0.tar.gz", "digest": "sha256:00",
                                        "file": str(other)}]),
        }
        for name, draft in cases.items():
            with self.subTest(name):
                self.draft(**draft)
                done = self.release()
                self.assertEqual(done.returncode, 1)
                self.assertIn("left as is", done.stderr)
                (found,) = self.gh()["releases"]
                self.assertTrue(found["draft"])
                self.assertEqual(self.writes(), [])

    def test_packaging_failure_leaves_no_remote_tag(self):
        self.s.git("rm", "-q", "skills/cicada/scripts/cicada_source.py")
        self.c = self.s.commit("Release 1.0.0 without the resolver")
        done = self.release()
        self.assertEqual(done.returncode, 1)
        self.assertIn("packaging the skill failed", done.stderr)
        self.assertIsNone(self.remote("refs/tags/v1.0.0"))
        self.assertEqual(self.writes(), [])

    def test_tag_object_mismatches_stop_publication(self):
        self.s.git("tag", "v1.0.0", "HEAD~1")
        self.assertIn("another commit", self.release().stderr)
        self.s.git("tag", "-d", "v1.0.0")
        # The remote holds an annotated tag at C; the local one is lightweight at the same commit.
        self.s.git("push", "-q", "--no-verify", "origin", f"{self.c}:refs/heads/main")
        self.s.git("tag", "-a", "v1.0.0", "-m", "v1.0.0", self.c)
        self.s.git("push", "-q", "--no-verify", "origin", "v1.0.0")
        self.s.git("tag", "-f", "v1.0.0", self.c)
        done = self.release()
        self.assertEqual(done.returncode, 1)
        self.assertIn("never replaced", done.stderr)
        self.assertEqual(self.writes(), [])

    def test_remote_main_that_contains_or_diverges(self):
        other = self.s.dir / "other"
        subprocess.run(["git", "clone", "-q", str(self.bare), str(other)], env=self.s.env, check=True)

        def push_from_other(base, name):
            subprocess.run(["git", "-C", str(other), "fetch", "-q", str(self.s.root), base], env=self.s.env,
                           check=True)
            subprocess.run(["git", "-C", str(other), "checkout", "-q", "-B", "work", "FETCH_HEAD"], env=self.s.env,
                           check=True)
            (other / name).write_text("x\n")
            subprocess.run(["git", "-C", str(other), "add", name], env=self.s.env, check=True)
            subprocess.run(["git", "-C", str(other), "commit", "-qm", name], env=self.s.env, check=True)
            subprocess.run(["git", "-C", str(other), "push", "-q", "-f", "origin", "work:main"], env=self.s.env,
                           check=True)

        push_from_other(self.s.git("rev-parse", "HEAD~1"), "diverged.txt")
        done = self.release()
        self.assertEqual(done.returncode, 1)
        self.assertIn("diverged", done.stderr)
        self.assertIsNone(self.remote("refs/tags/v1.0.0"))
        push_from_other(self.c, "ahead.txt")
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("already contains", done.stderr)
        self.assertEqual(self.remote("refs/tags/v1.0.0^{commit}"), self.c)

    def test_failing_checks_publish_nothing(self):
        fail = self.s.dir / "fail"
        fail.write_text("")
        self.c = self.s.commit("Failing e2e", {"scripts/gate-policy.json": json.dumps(policy(
            e2e=[flag_command("e2e", fail)]))})
        done = self.release()
        self.assertEqual(done.returncode, 1)
        self.assertIn("e2e failed", done.stderr)
        self.assertNotEqual(self.remote("refs/heads/main"), self.c)
        self.assertIsNone(self.remote("refs/tags/v1.0.0"))
        self.assertEqual(self.writes(), [])

    def test_release_check_pins_the_checked_out_commit(self):
        check = lambda *args: subprocess.run(["bash", "scripts/release-check.sh", *args], cwd=self.s.root,
                                             env=self.s.env, capture_output=True, text=True)
        before = check("--before-push", "v1.0.0")
        self.assertEqual((before.returncode, before.stdout), (0, NOTES), before.stderr)
        self.assertIn("not on main", check("v1.0.0").stderr, "the full mode fetches origin/main")
        self.s.git("tag", "v1.0.0", "HEAD~1")
        self.assertIn("not at the checked-out", check("--before-push", "v1.0.0").stderr)

    def test_a_rejected_main_push_is_refreshed_once(self):
        flag = self.s.dir / "reject-once"
        flag.write_text("")
        update = self.bare / "hooks" / "update"
        update.write_text(f'#!/bin/sh\nif [ -e "{flag}" ]; then rm "{flag}"; echo racing >&2; exit 1; fi\nexit 0\n')
        update.chmod(0o755)
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("rejected (attempt 1)", done.stderr)
        self.assert_published()


if __name__ == "__main__":
    unittest.main()
