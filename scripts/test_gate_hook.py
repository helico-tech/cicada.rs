"""The pre-push gate of scripts/gate.py against real pushes to temporary bare remotes, and its installation. The
hook is installed only in temporary repositories."""

from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate  # noqa: E402
from gate_testing import Sandbox  # noqa: E402


def remote_ref(bare: Path, ref: str, env) -> str | None:
    done = subprocess.run(["git", "--git-dir", str(bare), "rev-parse", "--verify", "-q", ref], capture_output=True,
                          text=True, env=env)
    return done.stdout.strip() or None


class InstallTests(unittest.TestCase):
    def test_install_writes_a_marked_hook_and_updates_only_its_own(self):
        sandbox = Sandbox(self)
        done = sandbox.gate("install-hook")
        self.assertEqual(done.returncode, 0, done.stderr)
        hook = sandbox.common / "hooks" / "pre-push"
        self.assertIn(gate.HOOK_MARKER, hook.read_text())
        self.assertIn("pre-push", hook.read_text())
        self.assertEqual(sandbox.gate("install-hook").returncode, 0, "reinstalling its own hook")

    def test_a_foreign_hook_is_refused_with_chaining_guidance(self):
        sandbox = Sandbox(self)
        hook = sandbox.common / "hooks" / "pre-push"
        hook.parent.mkdir(exist_ok=True)
        hook.write_text("#!/bin/sh\n# another tool\n")
        done = sandbox.gate("install-hook")
        self.assertEqual(done.returncode, 1)
        self.assertIn("Chain the gate", done.stderr)
        self.assertEqual(hook.read_text(), "#!/bin/sh\n# another tool\n")

    def test_core_hooks_path_is_respected_and_docs_hooks_stay(self):
        sandbox = Sandbox(self)
        sandbox.git("config", "core.hooksPath", "custom-hooks")
        docs_hook = sandbox.root / "custom-hooks" / "pre-commit"
        docs_hook.parent.mkdir()
        docs_hook.write_text("#!/bin/sh\n# cicada-docs-owner\nexit 0\n")
        done = sandbox.gate("install-hook")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn(gate.HOOK_MARKER, (sandbox.root / "custom-hooks" / "pre-push").read_text())
        self.assertFalse((sandbox.common / "hooks" / "pre-push").exists())
        self.assertEqual(docs_hook.read_text(), "#!/bin/sh\n# cicada-docs-owner\nexit 0\n")


class PushTests(unittest.TestCase):
    def setUp(self):
        self.s = Sandbox(self)
        self.bare = self.s.bare()
        self.s.git("push", "-q", "origin", "main")  # before the hook: the remote's accepted main
        self.s.git("fetch", "-q", "origin")
        self.assertEqual(self.s.gate("install-hook").returncode, 0)

    def push(self, *args, check=False):
        done = subprocess.run(["git", "push", *args], cwd=self.s.root, env=self.s.env, capture_output=True, text=True)
        if check:
            self.assertEqual(done.returncode, 0, done.stderr)
        return done

    def remote(self, ref):
        return remote_ref(self.bare, ref, self.s.env)

    def test_main_needs_evidence_for_its_tip(self):
        tip = self.s.commit("Code", {"src.rs": "1\n"})
        refused = self.push("origin", "main")
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("no passing checks", refused.stderr)
        self.assertNotEqual(self.remote("refs/heads/main"), tip)
        self.assertEqual(self.s.gate("run").returncode, 0)
        self.push("origin", "main", check=True)
        self.assertEqual(self.remote("refs/heads/main"), tip)

    def test_head_to_main_and_other_refs(self):
        self.s.git("switch", "-q", "-c", "feature")
        tip = self.s.commit("Feature", {"src.rs": "2\n"})
        self.push("origin", "feature", check=True)  # other refs pass without evidence
        self.assertNotEqual(self.push("origin", "HEAD:main").returncode, 0)
        self.assertEqual(self.s.gate("run").returncode, 0)
        self.push("origin", "HEAD:refs/heads/main", check=True)
        self.assertEqual(self.remote("refs/heads/main"), tip)

    def test_force_deletion_and_unknown_old_main_are_refused(self):
        start = self.s.git("rev-parse", "HEAD")
        self.assertEqual(self.s.gate("run").returncode, 0)
        tip = self.s.commit("Code", {"src.rs": "3\n"})
        self.assertEqual(self.s.gate("run").returncode, 0)
        self.push("origin", "main", check=True)
        self.s.git("fetch", "-q", "origin")
        refused = self.push("--force", "origin", f"{start}:refs/heads/main")
        self.assertIn("does not fast-forward", refused.stderr)
        self.assertIn("deleting main", self.push("origin", ":refs/heads/main").stderr)
        # Another clone moves the remote main to a commit unknown here.
        other = self.s.dir / "other"
        subprocess.run(["git", "clone", "-q", str(self.bare), str(other)], env=self.s.env, check=True)
        (other / "x.rs").write_text("x\n")
        subprocess.run(["git", "-C", str(other), "add", "x.rs"], env=self.s.env, check=True)
        subprocess.run(["git", "-C", str(other), "commit", "-qm", "Elsewhere"], env=self.s.env, check=True)
        subprocess.run(["git", "-C", str(other), "push", "-q", "origin", "main"], env=self.s.env, check=True)
        newer = self.s.commit("Mine", {"src.rs": "4\n"})
        self.assertEqual(self.s.gate("run").returncode, 0)
        refused = self.push("--force", "origin", "main")
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("unknown here", refused.stderr)
        self.assertNotIn(self.remote("refs/heads/main"), (newer, tip))

    def test_a_version_tag_needs_checks_e2e_and_accepted_main(self):
        tip = self.s.commit("Release", {"src.rs": "5\n"})
        self.s.git("tag", "-a", "v1.0.0", "-m", "v1.0.0")
        refused = self.push("origin", "v1.0.0")
        self.assertIn("no checks evidence", refused.stderr)
        self.assertEqual(self.s.gate("run").returncode, 0)
        self.assertIn("no e2e evidence", self.push("origin", "v1.0.0").stderr)
        self.assertEqual(self.s.gate("run", "--e2e").returncode, 0)
        self.assertIn("push main first", self.push("origin", "v1.0.0").stderr)
        self.assertIn("named remote", self.push(str(self.bare), "v1.0.0").stderr)
        self.s.git("update-ref", "-d", "refs/remotes/origin/main")
        self.assertIn("fetch origin first", self.push("origin", "v1.0.0").stderr)
        self.push("origin", "main", check=True)  # updates the tracking ref: now accepted main
        self.push("origin", "v1.0.0", check=True)
        self.assertEqual(self.remote("refs/tags/v1.0.0"), self.s.git("rev-parse", "v1.0.0"))
        self.assertEqual(self.remote("refs/tags/v1.0.0^{commit}"), tip)
        self.push("origin", "v1.0.0", check=True)  # up to date is no mutation

    def test_existing_remote_version_tags_never_change(self):
        tip = self.s.commit("Release", {"src.rs": "6\n"})
        self.assertEqual(self.s.gate("run", "--e2e").returncode, 0)
        self.push("origin", "main", check=True)
        self.s.git("tag", "-a", "v1.0.0", "-m", "v1.0.0")
        self.push("origin", "v1.0.0", check=True)
        published = self.remote("refs/tags/v1.0.0")
        # A lightweight tag at the same commit is another object.
        self.s.git("tag", "-f", "v1.0.0", tip)
        self.assertIn("never change", self.push("--force", "origin", "v1.0.0").stderr)
        self.assertIn("never change", self.push("origin", ":refs/tags/v1.0.0").stderr)
        self.assertEqual(self.remote("refs/tags/v1.0.0"), published)
        # Other tags are not gated.
        self.s.git("tag", "experiment")
        self.push("origin", "experiment", check=True)

    def test_a_tag_never_rides_on_a_main_update_the_server_may_reject(self):
        update = self.bare / "hooks" / "update"
        update.write_text('#!/bin/sh\n[ "$1" = refs/heads/main ] && { echo "main is protected" >&2; exit 1; }\nexit 0\n')
        update.chmod(0o755)
        accepted = self.s.git("rev-parse", "HEAD")
        candidate = self.s.commit("Release", {"src.rs": "7\n"})
        self.assertEqual(self.s.gate("run", "--e2e").returncode, 0, "the candidate main has full evidence")
        self.s.git("tag", "-a", "v2.0.0", "-m", "v2.0.0")
        for order in (["main", "v2.0.0"], ["v2.0.0", "main"], ["--follow-tags", "main"]):
            with self.subTest(order):
                refused = self.push("origin", *order)
                self.assertNotEqual(refused.returncode, 0)
                if order[0] != "--follow-tags":
                    self.assertIn("push main first", refused.stderr)
                self.assertIsNone(self.remote("refs/tags/v2.0.0"), "the server received no tag")
                self.assertEqual(self.remote("refs/heads/main"), accepted, "nor main")
        # Without the local gate this server takes the tag and rejects main: the partial push the gate prevents.
        bypass = self.push("--no-verify", "origin", "main", "v2.0.0")
        self.assertNotEqual(bypass.returncode, 0)
        self.assertEqual(self.remote("refs/tags/v2.0.0^{commit}"), candidate)
        self.assertEqual(self.remote("refs/heads/main"), accepted)


class ParseTests(unittest.TestCase):
    def test_every_update_is_read_before_deciding(self):
        sandbox = Sandbox(self)
        repo = gate.Repo(sandbox.root)
        head = sandbox.git("rev-parse", "HEAD")
        zero = "0" * 40
        errors = gate.pre_push(repo, "origin", [
            f"refs/heads/x {head} refs/heads/feature {zero}",
            f"refs/heads/main {head} refs/heads/main {zero}",
            "garbage",
            f"(delete) {zero} refs/tags/v1.2.3 {head}",
        ])
        self.assertEqual(len(errors), 3, errors)


if __name__ == "__main__":
    unittest.main()
