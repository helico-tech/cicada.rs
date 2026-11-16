"""The pinned release of scripts/gate.py against a temporary bare origin (reached through url.insteadOf, so origin
still names a GitHub repository) and a fake gh. No real push, tag or GitHub Release is made."""

import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tarfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from gate_testing import Sandbox, copy_script, flag_command, group_gone, install_fake_gh, policy  # noqa: E402

GITHUB = "https://github.com/acme/cicada.git"
FILES = {
    "Cargo.toml": '[workspace]\nmembers = ["c"]\nresolver = "2"\n\n[workspace.package]\nversion = "0.9.0"\n'
                  'repository = "https://github.com/acme/cicada"\n',
    "c/Cargo.toml": '[package]\nname = "c"\nversion.workspace = true\nedition = "2021"\n',
    "c/src/lib.rs": "",
    "skills/cicada/SKILL.md": "---\nname: cicada\nmetadata:\n  cicada-version: 0.9.0\n---\n# Skill\n",
    "skills/cicada/scripts/cicada_source.py": "print('resolver')\n",
    "skills/actor-design/SKILL.md": "---\nname: actor-design\nmetadata:\n  cicada-version: 0.9.0\n---\n# Design\n",
    "skills/actor-design/assets/design-template.md": "# Template\n",
    "scripts/skill-archive-format": "2\n",
    ".claude-plugin/plugin.json": '{"name": "cicada", "version": "0.9.0"}\n',
    "CHANGELOG.md": "# Changelog\n\n## 0.9.0 — 2026-11-01\n\n- Old.\n",
    **{f"fixtures/{name}": f'{{"format": 1, "name": "{name}"}}\n'
       for name in ("canonical.json", "contracts.json", "leases.json", "status.json")},
}
SKILL = "cicada-skill-v1.0.0.tar.gz"
FIXTURES = "cicada-fixtures-v1.0.0.tar.gz"
RELEASE = {
    "Cargo.toml": FILES["Cargo.toml"].replace("0.9.0", "1.0.0"),
    "skills/cicada/SKILL.md": FILES["skills/cicada/SKILL.md"].replace("0.9.0", "1.0.0"),
    "skills/actor-design/SKILL.md": FILES["skills/actor-design/SKILL.md"].replace("0.9.0", "1.0.0"),
    ".claude-plugin/plugin.json": FILES[".claude-plugin/plugin.json"].replace("0.9.0", "1.0.0"),
    "CHANGELOG.md": "# Changelog\n\n## 1.0.0 — 2026-11-15\n\n- First.\n\n## 0.9.0 — 2026-11-01\n\n- Old.\n",
}
NOTES = "\n- First.\n\n"
SKILL_NAMES = ["cicada", "cicada/SKILL.md", "cicada/SOURCE.json", "cicada/scripts", "cicada/scripts/cicada_source.py"]
DESIGN_NAMES = ["actor-design", "actor-design/SKILL.md", "actor-design/SOURCE.json", "actor-design/assets",
                "actor-design/assets/design-template.md"]


def legacy_release_check() -> str:
    """scripts/release-check.sh as before format 2 of the skill archive: without its format block."""
    lines = (Path(__file__).resolve().parent / "release-check.sh").read_text().split("\n")
    start = lines.index("# Format 2 of the skill archive: the format file and the second skill, in the same version.")
    end = lines.index("", start)
    return "\n".join(lines[:start] + lines[end + 1:])


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.s = Sandbox(self, files=FILES)
        copy_script(self.s, "release-check.sh")
        copy_script(self.s, "package-skill.py")
        copy_script(self.s, "package-fixtures.py")
        copy_script(self.s, "release_archive.py", 0o644)
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

    def assert_published(self, skill_names=DESIGN_NAMES + SKILL_NAMES):
        (found,) = self.gh()["releases"]
        self.assertFalse(found["draft"])
        self.assertEqual((found["tag_name"], found["name"], found["body"]), ("v1.0.0", "v1.0.0", NOTES))
        self.assertEqual(sorted(a["name"] for a in found["assets"]), [FIXTURES, SKILL])
        for asset in found["assets"]:
            data = Path(asset["file"]).read_bytes()
            with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
                names = archive.getnames()
                if asset["name"] == SKILL:
                    self.assertEqual(names, skill_names)
                for name in (n for n in names if n.endswith("/SOURCE.json")):
                    source = json.loads(archive.extractfile(name).read())
                    self.assertEqual((source["commit"], source["tag"]), (self.c, "v1.0.0"), name)
            self.assertEqual(self.state()["assets"][asset["name"]], hashlib.sha256(data).hexdigest())
        self.assertNotIn("asset_sha256", self.state())
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
            "other fixtures": dict(assets=[{"id": 9, "name": FIXTURES, "digest": "sha256:00", "file": str(other)}]),
            "repeated asset": dict(assets=[{"id": 9, "name": SKILL, "digest": "sha256:00", "file": str(other)},
                                           {"id": 10, "name": SKILL, "digest": "sha256:00", "file": str(other)}]),
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

    def test_a_failed_fixture_package_pushes_no_tag_and_saves_no_digest(self):
        self.s.git("rm", "-q", "fixtures/status.json")
        self.c = self.s.commit("Release 1.0.0 without the status fixture")
        done = self.release()
        self.assertEqual(done.returncode, 1)
        self.assertIn("packaging the fixtures failed", done.stderr)
        self.assertIsNone(self.remote("refs/tags/v1.0.0"))
        self.assertEqual(self.writes(), [])
        self.assertNotIn("assets", self.state())

    def interrupted_after_packaging(self) -> dict:
        """A release stopped by a failed draft creation: the tag is pushed and both digests are saved."""
        self.set_gh(fail=["create"])
        self.assertEqual(self.release().returncode, 1)
        state = self.state()
        self.assertEqual((state["stage"], sorted(state["assets"])), ("tag-pushed", [FIXTURES, SKILL]))
        self.set_gh(calls=[])
        return state

    def write_state(self, state: dict):
        (self.s.state / "release" / "v1.0.0.json").write_text(json.dumps(state))

    def test_a_retry_refuses_changed_saved_digests_or_names(self):
        state = self.interrupted_after_packaging()
        changes = {
            "other digest": {**state["assets"], FIXTURES: "0" * 64},
            "extra name": {**state["assets"], "cicada-extra-v1.0.0.tar.gz": "0" * 64},
            "missing name": {SKILL: state["assets"][SKILL]},
        }
        for name, assets in changes.items():
            with self.subTest(name):
                self.write_state({**state, "assets": assets})
                done = self.release()
                self.assertEqual(done.returncode, 1)
                self.assertIn("not the saved", done.stderr)
                self.assertEqual(self.state()["assets"], assets, "the saved map is never replaced")
                self.assertEqual(self.writes(), [])
        self.write_state(state)
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assert_published()

    def test_a_legacy_single_digest_state_is_refused_without_conversion(self):
        state = self.interrupted_after_packaging()
        legacy = {k: v for k, v in state.items() if k != "assets"} | {"asset_sha256": state["assets"][SKILL]}
        self.write_state(legacy)
        done = self.release()
        self.assertEqual(done.returncode, 1)
        self.assertIn("resume v1.0.0 with the scripts/gate.py of its pinned commit", done.stderr)
        self.assertEqual(self.state(), legacy)
        self.assertEqual(self.writes(), [])

    def assert_partial_draft_resumes(self, present: str, missing: str):
        state = self.interrupted_after_packaging()
        kept = self.s.dir / f"kept-{present}"
        kept.write_bytes((self.s.state / "release" / "v1.0.0" / present).read_bytes())
        self.draft(assets=[{"id": 9, "name": present, "digest": f"sha256:{state['assets'][present]}",
                            "file": str(kept)}])
        self.set_gh(calls=[])
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        uploads = [c for c in self.writes() if c[:2] == ["release", "upload"]]
        self.assertEqual([Path(c[3]).name for c in uploads], [missing], "only the missing asset is uploaded")
        self.assertFalse(any(c[:2] == ["release", "create"] for c in self.writes()))
        self.assert_published()

    def test_a_draft_with_only_the_skill_gets_the_fixtures(self):
        self.assert_partial_draft_resumes(SKILL, FIXTURES)

    def test_a_draft_with_only_the_fixtures_gets_the_skill(self):
        self.assert_partial_draft_resumes(FIXTURES, SKILL)

    def test_an_uncertain_upload_is_read_again_and_not_repeated(self):
        self.draft()
        self.set_gh(fail=["upload-after"])
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        uploads = [Path(c[3]).name for c in self.writes() if c[:2] == ["release", "upload"]]
        self.assertEqual(uploads, [SKILL, FIXTURES], "each missing asset once, the uncertain one read back")
        self.assert_published()

    def test_a_published_release_without_the_fixture_archive_is_left_untouched(self):
        self.assertEqual(self.release().returncode, 0)
        (found,) = self.gh()["releases"]
        found["assets"] = [a for a in found["assets"] if a["name"] == SKILL]  # as a release before this change
        self.set_gh(releases=[found], calls=[])
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("already published", done.stderr)
        self.assertEqual(self.writes(), [])
        self.assertEqual([a["name"] for a in self.gh()["releases"][0]["assets"]], [SKILL], "no backfill")

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
        self.assertFalse({"assets", "asset_sha256"} & set(self.state()), "a state from before packaging")
        push_from_other(self.c, "ahead.txt")
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("already contains", done.stderr)
        self.assertEqual(self.remote("refs/tags/v1.0.0^{commit}"), self.c)

    def test_a_killed_release_keeps_its_publisher_locked_until_recovery(self):
        self.set_gh(sleep="create")
        parent = self.s.spawn_gate("release", "v1.0.0")
        child = self.s.wait_for_child(timeout=120, argv_contains="release create")["child"]
        parent.kill()
        parent.wait()
        os.killpg(child["pgid"], 0)  # gh outlives the gate
        self.assertIn("busy", self.s.gate("recover").stderr, "the running gh keeps the lock")
        self.assertIn("busy", self.release().stderr, "no second publisher overlaps it")
        self.assertIsNotNone(self.s.marker())
        os.killpg(child["pgid"], signal.SIGKILL)
        self.assertTrue(group_gone(child["pgid"]))
        self.assertIn("abandoned", self.release().stderr)
        self.assertEqual(self.s.gate("recover").returncode, 0)
        self.set_gh(sleep=None)
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assert_published()

    def test_term_during_publication_stops_gh_and_blocks_until_recovery(self):
        self.set_gh(sleep="publish")
        parent = self.s.spawn_gate("release", "v1.0.0")
        child = self.s.wait_for_child(timeout=120, argv_contains="PATCH")["child"]
        parent.send_signal(signal.SIGTERM)
        parent.wait(timeout=90)
        self.assertEqual(parent.returncode, 130, self.s.spawned_log())
        self.assertTrue(group_gone(child["pgid"]))
        self.assertTrue(self.gh()["releases"][0]["draft"], "nothing was published")
        self.assertIn("abandoned", self.release().stderr)
        self.assertEqual(self.s.gate("recover").returncode, 0)
        self.set_gh(sleep=None)
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assert_published()

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

    def test_a_missing_malformed_or_stale_plugin_version_publishes_nothing(self):
        plugin = ".claude-plugin/plugin.json"
        cases = {
            "missing": (None, f"{plugin} is missing"),
            "malformed": ('{"version": ', "must be a JSON object with a string version"),
            "not an object": ('["1.0.0"]', "must be a JSON object with a string version"),
            "absent version": ('{"name": "cicada"}', "must be a JSON object with a string version"),
            "number version": ('{"version": 1.0}', "must be a JSON object with a string version"),
            "other version": ('{"version": "0.9.0"}', f"{plugin} says version 0.9.0"),
        }
        for name, (text, problem) in cases.items():
            with self.subTest(name):
                shutil.rmtree(self.s.state / "release", ignore_errors=True)  # each case is a first attempt
                if text is None:
                    self.s.git("rm", "-q", plugin)
                    self.c = self.s.commit(f"Release 1.0.0, plugin {name}")
                else:
                    self.c = self.s.commit(f"Release 1.0.0, plugin {name}", {plugin: text})
                done = self.release(self.c)
                self.assertEqual(done.returncode, 1)
                self.assertIn("release-check failed", done.stderr)
                self.assertIn(problem, done.stderr)
                self.assertNotEqual(self.remote("refs/heads/main"), self.c)
                self.assertIsNone(self.remote("refs/tags/v1.0.0"))
                self.assertEqual(self.writes(), [])

    def test_release_check_requires_format_two_and_both_skill_versions(self):
        design = "skills/actor-design/SKILL.md"
        stale = RELEASE[design].replace("1.0.0", "0.9.0")
        cases = {
            "no format file": ({"scripts/skill-archive-format": None}, "skill-archive-format is missing or does not"),
            "format 3": ({"scripts/skill-archive-format": "3\n"}, "does not hold 2 and a newline"),
            "no newline": ({"scripts/skill-archive-format": "2"}, "does not hold 2 and a newline"),
            "stale design skill": ({design: stale}, f"{design} says cicada-version 0.9.0"),
            "no design skill": ({design: None}, f"{design} says cicada-version nothing"),
        }
        for name, (files, problem) in cases.items():
            with self.subTest(name):
                shutil.rmtree(self.s.state / "release", ignore_errors=True)  # each case is a first attempt
                self.s.git("checkout", "-q", self.c, "--", ".")
                for path, text in files.items():
                    if text is None:
                        self.s.git("rm", "-q", "-f", path)
                    else:
                        self.s.write(path, text)
                commit = self.s.commit(f"Release 1.0.0, {name}")
                done = self.release(commit)
                self.assertEqual(done.returncode, 1)
                self.assertIn("release-check failed", done.stderr)
                self.assertIn(problem, done.stderr)
                self.assertNotEqual(self.remote("refs/heads/main"), commit)
                self.assertIsNone(self.remote("refs/tags/v1.0.0"))
                self.assertEqual(self.writes(), [])

    def test_a_legacy_commit_releases_and_resumes_the_single_skill_layout(self):
        for path in ("scripts/skill-archive-format", "skills/actor-design/SKILL.md",
                     "skills/actor-design/assets/design-template.md"):
            self.s.git("rm", "-q", path)
        self.c = self.s.commit("Release 1.0.0 as before format 2", {"scripts/release-check.sh": legacy_release_check()})
        state = self.interrupted_after_packaging()
        self.s.write("scripts/skill-archive-format", "2\n")  # the caller's working tree is never read
        self.s.write("skills/actor-design/SKILL.md", RELEASE["skills/actor-design/SKILL.md"])
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assert_published(SKILL_NAMES)
        self.assertEqual(self.state()["assets"], state["assets"], "the saved legacy digests are kept")

    def test_a_design_tree_without_the_format_file_is_refused_by_the_driver(self):
        self.s.git("rm", "-q", "scripts/skill-archive-format")
        self.c = self.s.commit("Release 1.0.0 without the format file",
                               {"scripts/release-check.sh": legacy_release_check()})
        done = self.release()
        self.assertEqual(done.returncode, 1)
        self.assertIn("has skills/actor-design but no scripts/skill-archive-format", done.stderr)
        self.assertIsNone(self.remote("refs/tags/v1.0.0"))
        self.assertNotIn("assets", self.state())
        self.assertEqual(self.writes(), [])

    def test_the_driver_checks_every_source_manifest_and_tagged_file(self):
        packager = (Path(__file__).resolve().parent / "package-skill.py").read_text()
        source = "        members[name] = (found.source(), 0o644)\n"
        files = "        members[name] = (files[path], mode)\n"
        design = "        if name.startswith('actor-design'):\n"
        cases = {
            "other design source": (source, design + "            members[name] = (found.source()"
                                    ".replace(b'v1.0.0', b'v0.9.0'), 0o644)\n",
                                    "actor-design/SOURCE.json in the skill archive names"),
            "empty cicada source": (source, "        if name.startswith('cicada/'):\n"
                                    "            members[name] = (b'{}', 0o644)\n",
                                    "cicada/SOURCE.json in the skill archive names {}"),
            "source as a folder": (source, design + "            members[name] = (None, 0o755)\n",
                                   "actor-design/SOURCE.json in the skill archive has the wrong type"),
            "other template": (files, "        if name.endswith('design-template.md'):\n"
                               "            members[name] = (b'other', mode)\n",
                               "actor-design/assets/design-template.md in the archive differs"),
        }
        for name, (after, extra, problem) in cases.items():
            with self.subTest(name):
                self.assertIn(after, packager)
                shutil.rmtree(self.s.state / "release", ignore_errors=True)  # each case is a first attempt
                self.s.git("checkout", "-q", self.c, "--", ".")
                commit = self.s.commit(f"Release 1.0.0, {name}",
                                       {"scripts/package-skill.py": packager.replace(after, after + extra)})
                done = self.release(commit)
                self.assertEqual(done.returncode, 1)
                self.assertIn(problem, done.stderr)
                self.assertIsNone(self.remote("refs/tags/v1.0.0"))
                self.assertEqual(self.writes(), [])
                self.assertNotIn("assets", self.state())
                self.s.git("tag", "-d", "v1.0.0")  # the local tag of the refused attempt, never pushed

    def test_a_format_two_retry_verifies_both_sources_against_the_pinned_commit(self):
        state = self.interrupted_after_packaging()
        # The caller's checkout moves on to a commit whose skills differ; the retry reads only the pinned commit.
        self.s.commit("Later skill edits", {"skills/actor-design/SKILL.md": "---\nname: actor-design\n---\nLater\n",
                                            "skills/actor-design/assets/design-template.md": "# Later\n"})
        changed = {**state["assets"], SKILL: "0" * 64}
        self.write_state({**state, "assets": changed})
        refused = self.release()
        self.assertEqual(refused.returncode, 1)
        self.assertIn("not the saved", refused.stderr)
        self.assertEqual(self.state()["assets"], changed, "the saved digests are never replaced")
        self.assertEqual(self.writes(), [])
        self.write_state(state)
        done = self.release()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assert_published()
        self.assertEqual(self.state()["assets"], state["assets"])

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
