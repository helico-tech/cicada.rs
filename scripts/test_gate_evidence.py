"""Evidence of scripts/gate.py in temporary repositories: the owned worktree, tree-bound records, the documentation
shortcut, the lock, interruption and recovery. Nothing here touches the repository the tests run in."""

import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate  # noqa: E402
from gate_testing import Sandbox, flag_command, group_gone, policy  # noqa: E402


def auth(sandbox, rev="HEAD"):
    repo = gate.Repo(sandbox.root)
    commit = repo.commit(rev)
    return gate.checks_auth(repo, commit), gate.e2e_auth(repo, commit)


class RunTests(unittest.TestCase):
    def test_run_records_a_pass_for_the_commit_not_the_working_tree(self):
        sandbox = Sandbox(self)
        sandbox.write("README.md", "# Dirty\n")
        sandbox.write("untracked.txt", "mine\n")
        sandbox.write("ignored.db", "mine\n")
        sandbox.write(".gitignore", "*.db\n")
        head = sandbox.git("rev-parse", "HEAD")
        done = sandbox.gate("run", "--e2e", env={"CICADA_GATE_PG_URL": "postgres://u:hunter2@db:5433/x?password=p2"})
        self.assertEqual(done.returncode, 0, done.stderr)
        record = sandbox.record("full")
        self.assertEqual((record["status"], record["commit"], record["kind"]), ("pass", head, "full"))
        self.assertEqual(record["tree"], sandbox.git("rev-parse", "HEAD^{tree}"))
        self.assertEqual(record["policy_blob"], sandbox.git("rev-parse", "HEAD:scripts/gate-policy.json"))
        self.assertEqual(record["runner_blob"], sandbox.git("rev-parse", "HEAD:scripts/gate.py"))
        self.assertEqual([c["exit_code"] for c in record["commands"]], [0])
        self.assertEqual(record["pg"], {"host": "db", "port": 5433, "database": "x"})
        self.assertIn("rustc", record["tools"])
        self.assertIn("finished_at", record)
        self.assertEqual(sandbox.record("e2e")["status"], "pass")
        for path in (sandbox.state / "records").iterdir():
            self.assertNotIn("hunter2", path.read_text())
            self.assertNotIn("p2", path.read_text().replace(record["policy_blob"], ""))
        self.assertEqual((sandbox.root / "README.md").read_text(), "# Dirty\n")
        self.assertEqual((sandbox.root / "untracked.txt").read_text(), "mine\n")
        self.assertTrue((sandbox.root / "ignored.db").exists())
        self.assertIsNone(sandbox.marker())
        owner = json.loads((sandbox.common / "worktrees" / "gate" / gate.OWNER_FILE).read_text())
        self.assertEqual(owner["path"], str(sandbox.owned))
        self.assertTrue(all(a.ok for a in auth(sandbox)))
        self.assertIn("checks: yes", sandbox.gate("status").stdout)

    def test_a_second_run_reuses_the_pass_and_force_reruns_it(self):
        sandbox = Sandbox(self)
        self.assertEqual(sandbox.gate("run").returncode, 0)
        started = sandbox.record("full")["started_at"]
        again = sandbox.gate("run")
        self.assertIn("checks of", again.stderr)
        self.assertEqual(sandbox.record("full")["started_at"], started)

    def test_forced_failure_replaces_the_pass_and_does_not_fall_back(self):
        sandbox = Sandbox(self)
        flag = sandbox.dir / "fail"
        sandbox.write("scripts/gate-policy.json", json.dumps(policy(full=[flag_command("flaky", flag)])))
        sandbox.commit("Flaky policy")
        self.assertEqual(sandbox.gate("run").returncode, 0)
        flag.write_text("")
        done = sandbox.gate("run", "--force")
        self.assertEqual(done.returncode, 1)
        self.assertEqual(sandbox.record("full")["status"], "failed")
        self.assertFalse(auth(sandbox)[0].ok)
        self.assertIn("full record: failed", sandbox.gate("status").stdout)
        self.assertIsNone(sandbox.marker(), "a normal failure clears the marker")
        flag.unlink()
        self.assertEqual(sandbox.gate("run").returncode, 0, "a failed record is rerun")
        self.assertTrue(auth(sandbox)[0].ok)

    def test_a_command_that_changes_its_source_fails(self):
        sandbox = Sandbox(self)
        flag = sandbox.dir / "write"
        flag.write_text("")
        sandbox.write("scripts/gate-policy.json", json.dumps(policy(full=[flag_command("writes", flag, "write")])))
        sandbox.commit("Writing policy")
        done = sandbox.gate("run")
        self.assertEqual(done.returncode, 1)
        self.assertIn("differs from the commit", done.stderr)
        self.assertEqual(sandbox.record("full")["status"], "failed")
        self.assertFalse(auth(sandbox)[0].ok)
        flag.unlink()
        self.assertEqual(sandbox.gate("run").returncode, 0, "the stray file is cleaned before the rerun")

    def test_the_runner_and_policy_must_be_the_commits_own(self):
        sandbox = Sandbox(self)
        sandbox.write("scripts/gate.py", (sandbox.root / "scripts/gate.py").read_text() + "\n# changed\n")
        done = sandbox.gate("run")
        self.assertEqual(done.returncode, 1)
        self.assertIn("differs from that commit's", done.stderr)
        sandbox.git("checkout", "--", "scripts/gate.py")
        sandbox.commit("Unsupported policy", {"scripts/gate-policy.json": json.dumps(dict(policy(), schema=9))})
        done = sandbox.gate("run")
        self.assertEqual(done.returncode, 1)
        self.assertIn("unsupported policy schema", done.stderr)

    def test_unowned_paths_are_left_untouched(self):
        sandbox = Sandbox(self)
        sandbox.owned.mkdir(parents=True)
        (sandbox.owned / "precious.txt").write_text("keep\n")
        done = sandbox.gate("run")
        self.assertEqual(done.returncode, 1)
        self.assertIn("left untouched", done.stderr)
        self.assertEqual((sandbox.owned / "precious.txt").read_text(), "keep\n")
        (sandbox.owned / "precious.txt").unlink()
        sandbox.owned.rmdir()
        sandbox.git("worktree", "add", "--detach", "-q", str(sandbox.owned), "HEAD")
        (sandbox.owned / "work.txt").write_text("someone's\n")
        done = sandbox.gate("run")
        self.assertEqual(done.returncode, 1, "a registered worktree without the owner marker")
        self.assertTrue((sandbox.owned / "work.txt").exists())

    def test_the_owned_path_must_be_ignored_and_target_must_be_a_real_cache(self):
        sandbox = Sandbox(self)
        (sandbox.common / "info" / "exclude").write_text("")
        done = sandbox.gate("run")
        self.assertEqual(done.returncode, 1)
        self.assertIn("must be ignored", done.stderr)
        (sandbox.common / "info" / "exclude").write_text(".claude/worktrees/\n")
        self.assertEqual(sandbox.gate("run").returncode, 0)
        (sandbox.owned / "target").mkdir()
        (sandbox.owned / "target" / "cache").write_text("kept\n")
        self.assertEqual(sandbox.gate("run", "--force").returncode, 0)
        self.assertTrue((sandbox.owned / "target" / "cache").exists(), "the target cache survives")
        (sandbox.owned / "target" / "cache").unlink()
        (sandbox.owned / "target").rmdir()
        (sandbox.owned / "target").symlink_to(sandbox.dir)
        self.assertEqual(sandbox.gate("run", "--force").returncode, 1)
        (sandbox.owned / "target").unlink()
        sandbox.commit("Tracked target", {"target/x": "x\n"})
        done = sandbox.gate("run")
        self.assertEqual(done.returncode, 1)
        self.assertIn("tracks target/", done.stderr)

    def test_external_cargo_and_toolchain_configuration_is_refused(self):
        sandbox = Sandbox(self)
        for path in (sandbox.dir / ".cargo" / "config.toml", sandbox.dir / "rust-toolchain.toml",
                     sandbox.cargo_home / "config.toml"):
            with self.subTest(str(path)):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("")
                done = sandbox.gate("run")
                self.assertEqual(done.returncode, 1)
                self.assertIn("from outside the commit", done.stderr)
                path.unlink()
        self.assertEqual(sandbox.gate("run").returncode, 0)

    def test_tampered_records_do_not_authorize(self):
        sandbox = Sandbox(self)
        self.assertEqual(sandbox.gate("run", "--e2e").returncode, 0)
        path = sandbox.state / "records" / f"full-{sandbox.git('rev-parse', 'HEAD^{tree}')}.json"
        good = json.loads(path.read_text())
        tampering = {
            "schema": lambda r: r.update(schema=7),
            "runner": lambda r: r.update(runner_blob="0" * 40),
            "policy": lambda r: r.update(policy_blob="0" * 40),
            "command missing": lambda r: r.update(commands=[]),
            "exit code": lambda r: r["commands"][0].update(exit_code=1),
            "other argv": lambda r: r["commands"][0].update(argv=["true"]),
            "commit": lambda r: r.update(commit="1" * 40),
            "status": lambda r: r.update(status="passed"),
            "unfinished": lambda r: r.pop("finished_at"),
        }
        for name, change in tampering.items():
            with self.subTest(name):
                record = json.loads(json.dumps(good))
                change(record)
                path.write_text(json.dumps(record))
                self.assertFalse(auth(sandbox)[0].ok)
        path.write_text("{")
        self.assertFalse(auth(sandbox)[0].ok)
        path.unlink()
        self.assertEqual(auth(sandbox)[0].how, "no checks evidence")


class ClassifierTests(unittest.TestCase):
    def setUp(self):
        self.sandbox = Sandbox(self, files={
            "docs/guide.md": "g\n", "docs/reference/cli.md": "c\n", "crates/x/README.md": "r\n",
            "openspec/config.yaml": "a: 1\n", "openspec/changes/x/proposal.md": "p\n", "src/lib.rs": "\n",
            "skills/cicada/SKILL.md": "s\n", ".claude/skills/build-loop/SKILL.md": "b\n", "CHANGELOG.md": "c\n",
        })
        self.repo = gate.Repo(self.sandbox.root)
        self.base = self.sandbox.git("rev-parse", "HEAD")
        self.allow = policy()["docs_allowlist"]

    def refused(self, files=None, prepare=None):
        self.sandbox.git("reset", "-q", "--hard", self.base)
        target = prepare() if prepare else None
        target = target or self.sandbox.commit("change", files)
        return gate.classify(self.repo, self.base, target, self.allow)

    def test_allowed_markdown_qualifies(self):
        for path in ("README.md", "CHANGELOG.md", "AGENTS.md", "docs/guide.md", "docs/new/deep.md",
                     "skills/cicada/SKILL.md", ".claude/skills/build-loop/SKILL.md", "openspec/changes/x/proposal.md",
                     "openspec/specs/y/spec.md"):
            with self.subTest(path):
                self.assertEqual(self.refused({path: "changed\n"}), [])
        self.assertEqual(self.refused(prepare=lambda: self.sandbox.git("rm", "-q", "docs/guide.md")), [])

    def test_other_inputs_force_full_checks(self):
        for path in ("docs/reference/cli.md", "crates/x/README.md", "openspec/config.yaml", "src/lib.rs",
                     "docs/openapi.json", "notes.txt", "docs/other.MD", ".reviews/x.taken"):
            with self.subTest(path):
                self.assertNotEqual(self.refused({path: "changed\n"}), [])

    def test_modes_symlinks_renames_and_submodules_force_full_checks(self):
        def executable():
            self.sandbox.write("docs/run.md", "x\n", 0o755)

        def mode_change():
            (self.sandbox.root / "docs/guide.md").chmod(0o755)

        def symlink():
            (self.sandbox.root / "docs/link.md").symlink_to("guide.md")

        def rename_out():
            self.sandbox.git("mv", "docs/guide.md", "guide.txt")

        def rename_in():
            self.sandbox.git("mv", "src/lib.rs", "docs/lib.md")

        def submodule():
            self.sandbox.git("update-index", "--add", "--cacheinfo", f"160000,{self.base},docs/sub.md")
            self.sandbox.git("commit", "-q", "-m", "submodule")
            return self.sandbox.git("rev-parse", "HEAD")

        for name, prepare in (("executable", executable), ("mode", mode_change), ("symlink", symlink),
                              ("rename out", rename_out), ("rename in", rename_in), ("submodule", submodule)):
            with self.subTest(name):
                self.assertNotEqual(self.refused(prepare=prepare), [])


class DocsReuseTests(unittest.TestCase):
    def test_docs_only_changes_run_docs_checks_over_a_direct_full_base(self):
        sandbox = Sandbox(self)
        base = sandbox.git("rev-parse", "HEAD")
        self.assertEqual(sandbox.gate("run", "--e2e").returncode, 0)
        sandbox.commit("Docs", {"docs/guide.md": "words\n"})
        done = sandbox.gate("run", "--e2e")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIsNone(sandbox.record("full"))
        docs = sandbox.record("docs")
        self.assertEqual((docs["status"], docs["base"]["commit"]), ("pass", base))
        self.assertIsNone(sandbox.record("e2e"), "e2e is reused from the base")
        checks, e2e = auth(sandbox)
        self.assertTrue(checks.ok and "docs pass" in checks.how)
        self.assertTrue(e2e.ok and "base" in e2e.how)
        # A second docs commit uses the direct full base again, never the derived docs record.
        sandbox.commit("More docs", {"README.md": "# More\n"})
        self.assertEqual(sandbox.gate("run").returncode, 0)
        self.assertEqual(sandbox.record("docs")["base"]["commit"], base)
        # A code change needs full checks.
        sandbox.commit("Code", {"src/lib.rs": "\n"})
        self.assertFalse(auth(sandbox)[0].ok)
        self.assertEqual(sandbox.gate("run").returncode, 0)
        self.assertEqual(sandbox.record("full")["status"], "pass")

    def test_failed_target_records_block_the_fallback(self):
        sandbox = Sandbox(self)
        self.assertEqual(sandbox.gate("run", "--e2e").returncode, 0)
        sandbox.commit("Docs", {"docs/guide.md": "words\n"})
        self.assertEqual(sandbox.gate("run", "--e2e").returncode, 0)
        repo = gate.Repo(sandbox.root)
        tree = sandbox.git("rev-parse", "HEAD^{tree}")
        for kind in ("full", "docs", "e2e"):
            with self.subTest(kind):
                path = repo.state / "records" / f"{kind}-{tree}.json"
                saved = path.read_text() if path.exists() else None
                path.write_text(json.dumps({"schema": 1, "kind": kind, "tree": tree, "status": "failed"}))
                checks, e2e = auth(sandbox)
                self.assertFalse(e2e.ok if kind == "e2e" else checks.ok)
                if saved is None:
                    path.unlink()
                else:
                    path.write_text(saved)

    def test_the_base_must_lie_within_100_first_parent_ancestors(self):
        sandbox = Sandbox(self)
        self.assertEqual(sandbox.gate("run").returncode, 0)
        repo = gate.Repo(sandbox.root)
        policy_blob, value, runner = gate.identity(repo, "HEAD")
        for n in range(gate.SEARCH_LIMIT):
            head = sandbox.commit(f"docs {n}", {"docs/log.md": f"{n}\n"})
        self.assertIsNotNone(gate.docs_base(repo, head, policy_blob, value, runner)[0], "100 back is found")
        head = sandbox.commit("one more", {"docs/log.md": "last\n"})
        base, why = gate.docs_base(repo, head, policy_blob, value, runner)
        self.assertIsNone(base)
        self.assertIn("100", why)

    def test_a_policy_or_runner_change_needs_full_checks(self):
        sandbox = Sandbox(self)
        self.assertEqual(sandbox.gate("run").returncode, 0)
        sandbox.commit("Policy", {"scripts/gate-policy.json": json.dumps(policy(docs=[
            {"name": "other", "argv": ["python3", "-c", "pass"]}]))})
        repo = gate.Repo(sandbox.root)
        self.assertIsNone(gate.docs_base(repo, repo.commit("HEAD"), *gate.identity(repo, "HEAD"))[0])


class LockTests(unittest.TestCase):
    def slow_sandbox(self, action="sleep"):
        sandbox = Sandbox(self)
        flag = sandbox.dir / "slow"
        flag.write_text("")
        sandbox.write("scripts/gate-policy.json", json.dumps(policy(full=[flag_command("slow", flag, action)])))
        sandbox.commit("Slow policy")
        return sandbox, flag

    def test_a_held_lock_refuses_a_run(self):
        sandbox = Sandbox(self)
        sandbox.state.mkdir(parents=True)
        fd = os.open(sandbox.state / "lock", os.O_RDWR | os.O_CREAT)
        self.addCleanup(os.close, fd)
        fcntl.flock(fd, fcntl.LOCK_EX)
        done = sandbox.gate("run")
        self.assertEqual(done.returncode, 1)
        self.assertIn("busy", done.stderr)
        self.assertIsNone(sandbox.record("full"))

    def test_a_killed_parent_leaves_its_child_holding_the_lock_and_an_abandoned_marker(self):
        sandbox, flag = self.slow_sandbox()
        parent = sandbox.spawn_gate("run")
        child = sandbox.wait_for_child()["child"]
        parent.kill()
        parent.wait()
        self.assertTrue(gate.group_alive(child["pgid"]))
        busy = sandbox.gate("run")
        self.assertIn("busy", busy.stderr, "the surviving child keeps the lock")
        self.assertIn("busy", sandbox.gate("recover").stderr)
        os.killpg(child["pgid"], signal.SIGKILL)
        self.assertTrue(group_gone(child["pgid"]))
        abandoned = sandbox.gate("run")
        self.assertEqual(abandoned.returncode, 1)
        self.assertIn("abandoned", abandoned.stderr)
        self.assertEqual(sandbox.record("full")["status"], "running")
        self.assertFalse(auth(sandbox)[0].ok)
        recovered = sandbox.gate("recover")
        self.assertEqual(recovered.returncode, 0, recovered.stderr)
        self.assertIsNone(sandbox.marker())
        self.assertEqual(sandbox.record("full")["status"], "running", "recovery never makes a pass")
        flag.unlink()
        self.assertEqual(sandbox.gate("run").returncode, 0)

    def test_term_stops_the_command_group_and_records_no_pass(self):
        sandbox, flag = self.slow_sandbox()
        parent = sandbox.spawn_gate("run")
        child = sandbox.wait_for_child()["child"]
        parent.send_signal(signal.SIGTERM)
        parent.wait(timeout=30)
        self.assertEqual(parent.returncode, 130, sandbox.spawned_log())
        self.assertIn("interrupted by SIGTERM", sandbox.spawned_log())
        self.assertTrue(group_gone(child["pgid"]))
        self.assertEqual(sandbox.record("full")["status"], "running")
        self.assertIsNotNone(sandbox.marker(), "cancellation does not declare its descendants recovered")
        self.assertEqual(sandbox.gate("recover").returncode, 0)

    def test_recover_stops_an_identified_group_that_released_the_lock(self):
        sandbox, flag = self.slow_sandbox("orphan")
        parent = sandbox.spawn_gate("run")
        child = sandbox.wait_for_child()["child"]
        time.sleep(0.3)
        parent.kill()
        parent.wait()
        self.assertTrue(gate.group_alive(child["pgid"]))
        done = sandbox.gate("recover")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(group_gone(child["pgid"]))
        self.assertIsNone(sandbox.marker())

    def test_recover_refuses_while_a_process_works_in_the_owned_worktree(self):
        sandbox, flag = self.slow_sandbox()
        parent = sandbox.spawn_gate("run")
        child = sandbox.wait_for_child()["child"]
        parent.kill()
        parent.wait()
        os.killpg(child["pgid"], signal.SIGKILL)
        self.assertTrue(group_gone(child["pgid"]))
        squatter = subprocess.Popen(["sleep", "60"], cwd=sandbox.owned)
        self.addCleanup(squatter.wait)
        self.addCleanup(squatter.kill)
        done = sandbox.gate("recover")
        self.assertEqual(done.returncode, 1)
        self.assertIn("still work in", done.stderr)
        self.assertIsNotNone(sandbox.marker())


if __name__ == "__main__":
    unittest.main()
