"""Tests of skills/cicada/scripts/cicada_source.py, run by examples/tests/skill.rs (or `python3 -m unittest` here).

Each test installs a copy of the skill in a temporary folder, as a consumer would, and runs the helper as a program:
on saved metadata against temporary source trees, with fake `cargo` and `rustc` programs that record their calls, or
with the real Cargo on small temporary projects that need no network.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
SKILL = REPO / "skills" / "cicada"
GIT = ["git", "-c", "user.name=test", "-c", "user.email=test@example.test", "-c", "commit.gpgsign=false"]
UPSTREAM = "https://example.test/cicada.rs"


def git(cwd: Path, *args: str) -> str:
    return subprocess.run([*GIT, *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def install_skill(folder: Path, version: str = "0.17.2", source: dict | None = None) -> Path:
    """A copy of the skill at folder/skills/cicada with `version` in its frontmatter; returns the helper's path."""
    target = folder / "skills" / "cicada"
    (target / "scripts").mkdir(parents=True)
    text = (SKILL / "SKILL.md").read_text(encoding="utf-8")
    lines = [f"  cicada-version: {version}" if line.startswith("  cicada-version:") else line for line in text.split("\n")]
    (target / "SKILL.md").write_text("\n".join(lines), encoding="utf-8")
    shutil.copy(SKILL / "scripts" / "cicada_source.py", target / "scripts" / "cicada_source.py")
    if source is not None:
        (target / "SOURCE.json").write_text(json.dumps(source), encoding="utf-8")
    return target / "scripts" / "cicada_source.py"


def make_tree(root: Path, version: str = "0.17.2", names=("cicada", "cicada-test"), commit: bool = False) -> str | None:
    """A Cicada source tree: crates/<name>/Cargo.toml with src/lib.rs, and docs/architecture.md."""
    for name in names:
        crate = root / "crates" / name
        (crate / "src").mkdir(parents=True, exist_ok=True)
        (crate / "Cargo.toml").write_text(
            f'[package]\nname = "{name}"\nversion = "{version}"\nedition = "2021"\n', encoding="utf-8"
        )
        (crate / "src" / "lib.rs").write_text("", encoding="utf-8")
    (root / "docs").mkdir(parents=True, exist_ok=True)
    (root / "docs" / "architecture.md").write_text("# Architecture\n", encoding="utf-8")
    if not commit:
        return None
    git(root, "init", "-q")
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", "tree")
    return git(root, "rev-parse", "HEAD")


def package(name: str, version: str, manifest: Path, source: str | None = None) -> dict:
    ident = f"{source}#{name}@{version}" if source else f"path+file://{manifest.parent}#{name}@{version}"
    return {"name": name, "version": version, "id": ident, "source": source, "manifest_path": str(manifest)}


def metadata(packages: list[dict], workspace: Path, resolved: list[dict] | None = None, members=()) -> dict:
    """Cargo metadata: every package listed, the `resolved` ones (default: all) in the filtered graph."""
    nodes = [{"id": p["id"], "dependencies": []} for p in (packages if resolved is None else resolved)]
    return {
        "packages": packages,
        "resolve": {"nodes": nodes, "root": None},
        "workspace_members": [p["id"] for p in members],
        "workspace_root": str(workspace),
        "version": 1,
    }


def run(script: Path, *args: str, cwd: Path, env: dict | None = None) -> dict:
    done = subprocess.run(
        [sys.executable, "-I", str(script), *args],
        cwd=cwd,
        env=env if env is not None else dict(os.environ),
        capture_output=True,
        text=True,
        timeout=120,
    )
    if done.returncode != 0:
        raise AssertionError(f"the helper exited with {done.returncode}: {done.stderr}")
    return json.loads(done.stdout)


def snapshot(folder: Path) -> dict:
    """Every file under folder with its bytes, to show that nothing was written."""
    return {str(p.relative_to(folder)): p.read_bytes() for p in sorted(folder.rglob("*")) if p.is_file()}


class Fixtures(unittest.TestCase):
    """Saved metadata (--metadata) against temporary trees."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.app = self.dir / "app"
        self.app.mkdir()

    def resolve(self, data: dict, script: Path | None = None, *args: str, env: dict | None = None) -> dict:
        path = self.dir / f"metadata-{len(list(self.dir.glob('metadata-*')))}.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        script = script or install_skill(self.dir / f"install-{path.stem}")
        args = ("--metadata", str(path), "--target", "x86_64-unknown-linux-gnu", *args)
        return run(script, *args, cwd=self.app, env=env)

    def git_packages(self, tree: Path, revision: str, version: str = "0.17.2", tag: str = "v0.17.2") -> list[dict]:
        source = f"git+{UPSTREAM}?tag={tag}#{revision}"
        return [package(n, version, tree / "crates" / n / "Cargo.toml", source) for n in ("cicada", "cicada-test")]

    def consumer(self) -> dict:
        return package("cicada-code-factory", "0.1.0", self.app / "Cargo.toml")

    def test_a_locked_git_dependency_reports_its_full_identity(self):
        tree = self.dir / "checkout"
        head = make_tree(tree, commit=True)
        (tree / ".cargo-ok").write_text("ok", encoding="utf-8")  # Cargo's marker is no change
        me = self.consumer()
        report = self.resolve(metadata([me, *self.git_packages(tree, head)], self.app, members=[me]))
        self.assertEqual(report["status"], "ok")
        self.assertEqual(report["target"], "x86_64-unknown-linux-gnu")
        self.assertEqual([p["name"] for p in report["packages"]], ["cicada", "cicada-test"])
        [group] = report["groups"]
        self.assertEqual(group["kind"], "git")
        self.assertEqual(group["url"], UPSTREAM)
        self.assertEqual(group["selector"], {"kind": "tag", "value": "v0.17.2"})
        self.assertEqual(group["revision"], head)
        self.assertEqual(group["commit"], head)
        self.assertEqual(group["root"], str(tree))
        self.assertIs(group["dirty"], False)
        self.assertIs(group["head_matches_revision"], True)
        self.assertIs(group["modified"], False)
        self.assertIs(group["docs_available"], True)
        self.assertEqual(report["packages"][0]["manifest_path"], str(tree / "crates" / "cicada" / "Cargo.toml"))

    def test_a_dirty_checkout_or_a_moved_head_is_modified(self):
        tree = self.dir / "checkout"
        head = make_tree(tree, commit=True)
        (tree / "docs" / "architecture.md").write_text("# Changed\n", encoding="utf-8")
        report = self.resolve(metadata(self.git_packages(tree, head), self.app))
        self.assertIs(report["groups"][0]["dirty"], True)
        self.assertIs(report["groups"][0]["modified"], True)

        other = "0" * 40
        report = self.resolve(metadata(self.git_packages(tree, other), self.app))
        group = report["groups"][0]
        self.assertEqual((group["revision"], group["commit"]), (other, head))
        self.assertIs(group["head_matches_revision"], False)
        self.assertIs(group["modified"], True)

    def test_a_path_dependency_outside_git_is_unversioned(self):
        tree = self.dir / "cicada.rs"
        make_tree(tree)
        report = self.resolve(metadata([package("cicada", "0.17.2", tree / "crates/cicada/Cargo.toml")], self.app))
        self.assertEqual(report["status"], "ok")
        [group] = report["groups"]
        self.assertEqual((group["kind"], group["root"], group["source"]), ("path", str(tree), None))
        self.assertIs(group["unversioned"], True)
        self.assertIsNone(group["commit"])
        self.assertEqual(group["revision_match"], "unknown")
        self.assertIsNone(group["modified"])

    def test_a_patched_fork_of_the_same_version_is_mixed(self):
        upstream, fork = self.dir / "checkout", self.dir / "fork"
        head = make_tree(upstream, commit=True)
        make_tree(fork, commit=True)
        packages = [
            *self.git_packages(upstream, head)[:1],
            package("cicada-test", "0.17.2", fork / "crates/cicada-test/Cargo.toml"),
        ]
        report = self.resolve(metadata(packages, self.app))
        self.assertEqual(report["status"], "mixed")
        self.assertEqual(sorted(g["kind"] for g in report["groups"]), ["git", "path"])
        self.assertEqual({g["version"] for g in report["groups"]}, {"0.17.2"})
        self.assertEqual(len({p["group"] for p in report["packages"]}), 2)

    def test_two_tags_are_mixed(self):
        old, new = self.dir / "old", self.dir / "new"
        old_head, new_head = make_tree(old, "0.16.0", commit=True), make_tree(new, commit=True)
        packages = [*self.git_packages(old, old_head, "0.16.0", "v0.16.0"), *self.git_packages(new, new_head)]
        report = self.resolve(metadata(packages, self.app))
        self.assertEqual(report["status"], "mixed")
        self.assertEqual(sorted(g["version"] for g in report["groups"]), ["0.16.0", "0.17.2"])

    def test_paths_without_a_tree_stay_apart(self):
        a, b = self.dir / "a" / "cicada", self.dir / "b" / "cicada"
        packages = [package("cicada", "0.17.2", a / "Cargo.toml"), package("cicada-test", "0.17.2", b / "Cargo.toml")]
        report = self.resolve(metadata(packages, self.app))
        self.assertEqual(report["status"], "mixed")
        self.assertEqual(sorted(g["manifest_folder"] for g in report["groups"]), [str(a), str(b)])

    def test_the_cicada_workspace_itself_is_self(self):
        tree = self.dir / "cicada.rs"
        make_tree(tree)
        core = package("cicada", "0.17.2", tree / "crates/cicada/Cargo.toml")
        report = self.resolve(metadata([core], tree, members=[core]))
        self.assertEqual(report["status"], "self")

    def test_a_member_named_cicada_outside_a_cicada_tree_is_not_self(self):
        own = package("cicada", "0.1.0", self.app / "Cargo.toml")
        report = self.resolve(metadata([own], self.app, members=[own]))
        self.assertEqual(report["status"], "no-docs")
        self.assertEqual(report["packages"][0]["manifest_path"], str(self.app / "Cargo.toml"))

    def test_a_consumer_whose_name_starts_with_cicada_is_none(self):
        me = self.consumer()
        report = self.resolve(metadata([me], self.app, members=[me]))
        self.assertEqual((report["status"], report["packages"], report["groups"]), ("none", [], []))

    def test_a_vendored_package_without_docs_is_kept(self):
        vendor = self.dir / "vendor" / "cicada"
        vendor.mkdir(parents=True)
        source = f"git+{UPSTREAM}?tag=v0.17.2#{'a' * 40}"
        report = self.resolve(metadata([package("cicada", "0.17.2", vendor / "Cargo.toml", source)], self.app))
        self.assertEqual(report["status"], "no-docs")
        [group] = report["groups"]
        self.assertIs(group["docs_available"], False)
        self.assertEqual((group["source"], group["revision"]), (source, "a" * 40))

    def test_packages_outside_the_filtered_graph_are_excluded(self):
        tree = self.dir / "cicada.rs"
        make_tree(tree, names=("cicada", "cicada-otel"))
        core = package("cicada", "0.17.2", tree / "crates/cicada/Cargo.toml")
        otel = package("cicada-otel", "0.17.1", tree / "crates/cicada-otel/Cargo.toml")
        report = self.resolve(metadata([core, otel], self.app, resolved=[core]))
        self.assertEqual(report["status"], "ok")
        self.assertEqual([p["name"] for p in report["packages"]], ["cicada"])

    def test_an_equal_version_with_another_commit_is_no_match(self):
        tree = self.dir / "checkout"
        head = make_tree(tree, commit=True)
        data = metadata(self.git_packages(tree, head), self.app)
        released = {"repository": UPSTREAM, "tag": "v0.17.2", "commit": "f" * 40, "version": "0.17.2"}
        group = self.resolve(data, install_skill(self.dir / "released", source=released))["groups"][0]
        self.assertEqual((group["version_relation"], group["revision_match"]), ("equal", "different"))

        same = dict(released, commit=head)
        report = self.resolve(data, install_skill(self.dir / "same", source=same))
        self.assertEqual(report["skill_source"], same)
        self.assertEqual(report["groups"][0]["revision_match"], "same")

        copied = self.resolve(data, install_skill(self.dir / "copied"))  # no SOURCE.json
        self.assertIsNone(copied["skill_source"])
        self.assertEqual(copied["groups"][0]["revision_match"], "unknown")

    def released_checkout(self) -> tuple[dict, Path]:
        """Saved metadata of a git checkout whose locked revision and HEAD are the commit of the installed skill."""
        tree = self.dir / "checkout"
        head = make_tree(tree, commit=True)
        released = {"repository": UPSTREAM, "tag": "v0.17.2", "commit": head, "version": "0.17.2"}
        return metadata(self.git_packages(tree, head), self.app), install_skill(self.dir / "released", source=released)

    def assert_unproven(self, group: dict):
        self.assertEqual(group["revision_match"], "unknown")
        self.assertIsNone(group["modified"])

    def test_a_missing_git_proves_no_match(self):
        data, script = self.released_checkout()
        self.assertEqual(self.resolve(data, script)["groups"][0]["revision_match"], "same")  # with git
        empty = self.dir / "no-tools"
        empty.mkdir()
        group = self.resolve(data, script, env=dict(os.environ, PATH=str(empty)))["groups"][0]
        self.assert_unproven(group)
        self.assertEqual((group["commit"], group["dirty"], group["unversioned"]), (None, None, None))
        self.assertEqual(group["revision"], data["packages"][0]["source"].rsplit("#", 1)[1], "Cargo's revision stays")

    def test_a_failing_git_probe_proves_no_match(self):
        data, script = self.released_checkout()
        real = shutil.which("git")
        fake = self.dir / "fake-git"
        fake.mkdir()
        (fake / "git").write_text(
            f"#!{sys.executable}\n"
            "import os, sys\n"
            "args = sys.argv[1:]\n"
            "fail = os.environ['FAKE_GIT_FAIL']\n"
            "if (fail == 'HEAD' and 'HEAD' in args) or (fail == 'status' and 'status' in args):\n"
            "    sys.exit(128)\n"
            f"os.execv({real!r}, [{real!r}, *args])\n",
            encoding="utf-8",
        )
        (fake / "git").chmod(0o755)
        for fail in ("HEAD", "status"):
            env = dict(os.environ, PATH=f"{fake}{os.pathsep}{os.environ['PATH']}", FAKE_GIT_FAIL=fail)
            group = self.resolve(data, script, env=env)["groups"][0]
            self.assert_unproven(group)
            self.assertIs(group["unversioned"], False, fail)
            self.assertIsNone(group["commit" if fail == "HEAD" else "dirty"], fail)

    def test_versions_compare_with_the_skill(self):
        tree = self.dir / "cicada.rs"
        make_tree(tree, "0.16.3")
        data = metadata([package("cicada", "0.16.3", tree / "crates/cicada/Cargo.toml")], self.app)
        for skill, relation in [("0.17.2", "older"), ("0.16.3", "equal"), ("0.16.0", "newer"), ("next", "unknown")]:
            report = self.resolve(data, install_skill(self.dir / f"skill-{skill}", version=skill))
            self.assertEqual(report["groups"][0]["version_relation"], relation, skill)

    def test_metadata_without_a_resolve_graph_is_unavailable(self):
        data = metadata([], self.app)
        data["resolve"] = None
        report = self.resolve(data)
        self.assertEqual((report["status"], report["reason"]), ("unavailable", "bad-metadata"))


FAKE = """#!{python}
import json, os, sys, time
name = os.path.basename(sys.argv[0])
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write(json.dumps({{"tool": name, "args": sys.argv[1:], "rustup_auto_install": os.environ.get("RUSTUP_AUTO_INSTALL")}}) + "\\n")
mode = os.environ.get("FAKE_MODE", "")
if name == "rustc":
    if mode == "no-toolchain":
        sys.stderr.write("error: toolchain 'stable-x86_64-unknown-linux-gnu' is not installed\\n")
        sys.exit(1)
    print("rustc 1.94.0\\nhost: x86_64-unknown-linux-gnu")
    sys.exit(0)
if "locate-project" in sys.argv:
    print(os.path.join(os.getcwd(), "Cargo.toml"))
    sys.exit(0)
if mode == "sleep":
    time.sleep(30)
if mode == "metadata-fail":
    sys.stderr.write("error: failed to download `cicada`\\n")
    sys.exit(101)
print(json.dumps({{"packages": [], "resolve": {{"nodes": []}}, "workspace_members": [], "workspace_root": os.getcwd()}}))
"""


class FakeTools(unittest.TestCase):
    """Fake cargo and rustc programs that record each call."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.bin = self.dir / "bin"
        self.bin.mkdir()
        for tool in ("cargo", "rustc"):
            path = self.bin / tool
            path.write_text(FAKE.format(python=sys.executable), encoding="utf-8")
            path.chmod(0o755)
        self.project = self.dir / "project"
        self.project.mkdir()
        (self.project / "Cargo.toml").write_text('[package]\nname = "app"\nversion = "0.1.0"\n', encoding="utf-8")
        (self.project / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
        self.log = self.dir / "calls.log"
        self.script = install_skill(self.dir / "install")

    def resolve(self, mode: str = "", *args: str) -> dict:
        env = dict(os.environ, PATH=str(self.bin), FAKE_LOG=str(self.log), FAKE_MODE=mode)
        before = snapshot(self.project)
        report = run(self.script, *args, cwd=self.project, env=env)
        self.assertEqual(snapshot(self.project), before, "the project is unchanged")
        return report

    def calls(self) -> list[dict]:
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def assert_read_only(self):
        for call in self.calls():
            self.assertEqual(call["rustup_auto_install"], "0", call)
            self.assertFalse({"fetch", "update", "generate-lockfile", "install"} & set(call["args"]), call)
        metadata = [c for c in self.calls() if c["tool"] == "cargo" and "metadata" in c["args"]]
        self.assertLessEqual(len(metadata), 1, "no retry")
        for call in metadata:
            for flag in ("--locked", "--offline", "--filter-platform"):
                self.assertIn(flag, call["args"])
            self.assertNotIn("--no-deps", call["args"])

    def test_the_host_target_and_the_features_are_passed_and_reported(self):
        report = self.resolve("", "--no-default-features", "--features", "postgres", "--features", "tui")
        self.assertEqual(report["status"], "none")
        self.assertEqual(report["target"], "x86_64-unknown-linux-gnu")
        self.assertEqual(
            report["features"], {"all_features": False, "no_default_features": True, "features": ["postgres", "tui"]}
        )
        [call] = [c for c in self.calls() if "metadata" in c["args"]]
        args = call["args"]
        self.assertEqual(args[args.index("--filter-platform") + 1], "x86_64-unknown-linux-gnu")
        self.assertIn("--no-default-features", args)
        self.assertEqual([args[i + 1] for i, a in enumerate(args) if a == "--features"], ["postgres", "tui"])
        self.assert_read_only()

    def test_an_explicit_target_skips_rustc_and_all_features_is_passed(self):
        report = self.resolve("", "--target", "aarch64-apple-darwin", "--all-features")
        self.assertEqual(report["target"], "aarch64-apple-darwin")
        self.assertEqual([c["tool"] for c in self.calls()], ["cargo", "cargo"])
        self.assertIn("--all-features", self.calls()[-1]["args"])
        self.assert_read_only()

    def test_a_failed_metadata_is_unavailable_without_a_retry(self):
        report = self.resolve("metadata-fail")
        self.assertEqual((report["status"], report["reason"]), ("unavailable", "metadata-failed"))
        self.assertIn("failed to download", report["diagnostic"])
        self.assertIn("cargo fetch --locked", report["hint"])
        self.assertNotIn("cargo update", report["hint"])
        self.assert_read_only()

    def test_a_missing_toolchain_is_unavailable_and_not_installed(self):
        report = self.resolve("no-toolchain")
        self.assertEqual((report["status"], report["reason"]), ("unavailable", "rustc-failed"))
        self.assertIn("not installed", report["diagnostic"])
        self.assert_read_only()

    def test_a_slow_cargo_times_out(self):
        report = self.resolve("sleep", "--timeout", "1")
        self.assertEqual((report["status"], report["reason"]), ("unavailable", "timeout"))
        self.assert_read_only()

    def test_a_missing_cargo_is_unavailable(self):
        (self.bin / "cargo").unlink()
        report = self.resolve()
        self.assertEqual((report["status"], report["reason"]), ("unavailable", "missing-tool"))
        self.assertIn("cargo", report["diagnostic"])

    def test_a_missing_lockfile_is_unavailable_and_not_written(self):
        (self.project / "Cargo.lock").unlink()
        report = self.resolve()
        self.assertEqual((report["status"], report["reason"]), ("unavailable", "missing-lock"))
        self.assertFalse((self.project / "Cargo.lock").exists())
        self.assertFalse([c for c in self.calls() if "metadata" in c["args"]])


@unittest.skipUnless(shutil.which("cargo"), "needs cargo")
class RealCargo(unittest.TestCase):
    """The real Cargo on temporary projects whose dependencies are local, so no network is needed."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.script = install_skill(self.dir / "install")
        self.env = dict(os.environ, CARGO_HOME=str(self.dir / "cargo-home"), CARGO_TARGET_DIR=str(self.dir / "target"))

    def cargo(self, *args: str, cwd: Path):
        subprocess.run(["cargo", *args], cwd=cwd, env=self.env, check=True, capture_output=True, timeout=300)

    def test_inherited_dependencies_features_and_targets_resolve_offline(self):
        upstream = self.dir / "cicada.rs"
        make_tree(upstream, names=("cicada", "cicada-otel", "cicada-actors"))
        app = self.dir / "cicada-code-factory"
        (app / "src").mkdir(parents=True)
        (app / "src" / "lib.rs").write_text("", encoding="utf-8")
        (app / "Cargo.toml").write_text(
            f"""[workspace]

[workspace.dependencies]
cicada = {{ path = "{upstream}/crates/cicada" }}

[package]
name = "cicada-code-factory"
version = "0.1.0"
edition = "2021"

[features]
default = ["otel"]
otel = ["dep:cicada-otel"]

[dependencies]
cicada = {{ workspace = true }}
cicada-otel = {{ path = "{upstream}/crates/cicada-otel", optional = true }}

[target.'cfg(any())'.dependencies]
cicada-actors = {{ path = "{upstream}/crates/cicada-actors" }}
""",
            encoding="utf-8",
        )
        self.cargo("generate-lockfile", "--offline", cwd=app)
        before = snapshot(app)

        def names(*args: str) -> list[str]:
            report = run(self.script, *args, cwd=app, env=self.env)
            self.assertEqual(report["status"], "ok", report)
            self.assertEqual(report["groups"][0]["root"], str(upstream))
            return [p["name"] for p in report["packages"]]

        self.assertEqual(names(), ["cicada", "cicada-otel"])
        self.assertEqual(names("--no-default-features"), ["cicada"])
        self.assertEqual(names("--no-default-features", "--features", "otel"), ["cicada", "cicada-otel"])
        manifest = run(self.script, "--manifest-path", str(app / "Cargo.toml"), cwd=self.dir, env=self.env)
        self.assertEqual(manifest["workspace"], str(app))
        self.assertEqual(snapshot(app), before, "no project file changed")

    def test_a_fetched_git_tag_resolves_to_its_checkout(self):
        upstream = self.dir / "upstream"
        head = make_tree(upstream, commit=True)
        git(upstream, "tag", "v0.17.2")
        app = self.dir / "app"
        (app / "src").mkdir(parents=True)
        (app / "src" / "lib.rs").write_text("", encoding="utf-8")
        url = upstream.as_uri()
        (app / "Cargo.toml").write_text(
            f'[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n'
            f'cicada = {{ git = "{url}", tag = "v0.17.2" }}\ncicada-test = {{ git = "{url}", tag = "v0.17.2" }}\n',
            encoding="utf-8",
        )
        self.cargo("generate-lockfile", cwd=app)  # a local file URL: no network
        self.cargo("fetch", "--locked", cwd=app)
        before = snapshot(app)
        report = run(self.script, cwd=app, env=self.env)
        self.assertEqual(report["status"], "ok", report)
        [group] = report["groups"]
        self.assertEqual(group["kind"], "git")
        self.assertEqual(group["selector"], {"kind": "tag", "value": "v0.17.2"})
        self.assertEqual((group["revision"], group["commit"]), (head, head))
        self.assertIs(group["dirty"], False)
        self.assertTrue(group["root"].startswith(str(self.dir / "cargo-home")), group["root"])
        self.assertEqual(group["packages"], ["cicada", "cicada-test"])
        self.assertEqual(snapshot(app), before)

    def test_a_project_without_a_lockfile_gets_none_written(self):
        app = self.dir / "app"
        (app / "src").mkdir(parents=True)
        (app / "src" / "lib.rs").write_text("", encoding="utf-8")
        (app / "Cargo.toml").write_text('[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n')
        report = run(self.script, cwd=app, env=self.env)
        self.assertEqual((report["status"], report["reason"]), ("unavailable", "missing-lock"))
        self.assertFalse((app / "Cargo.lock").exists())


if __name__ == "__main__":
    unittest.main()
