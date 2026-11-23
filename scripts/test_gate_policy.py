"""The check policy of scripts/gate.py: its schema, the controlled environment, parity with the former CI workflow
and the manual workflow's triggers. No real publication tool is invoked."""

from fnmatch import fnmatch
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate  # noqa: E402
from gate_testing import REAL_POLICY, SCRIPTS, Sandbox, policy  # noqa: E402

ROOT = SCRIPTS.parent

# Every command of .github/workflows/ci.yml before ADR 0046, in order; each must stay in the full policy.
FORMER_CI = [
    "cargo fmt --all --check",
    "cargo clippy --workspace --all-targets -- -D warnings",
    "cargo test --workspace",
    "cargo check -p cicada",
    "cargo check -p cicada --features sqlite",
    "cargo check -p cicada --features postgres",
    "cargo check -p cicada --features testing",
    "cargo check -p cicada-app",
    "cargo check -p cicada-app --features sqlite",
    "cargo check -p cicada-app --features postgres",
    "cargo check -p cicada-app --features tui",
    "cargo test -p cicada-app",
    'cargo tree -p cicada-app -e normal > "$trees/app.txt"',
    'cargo tree -p cicada-app -e normal --features tui > "$trees/app-tui.txt"',
    "grep -q 'ratatui v' \"$trees/app-tui.txt\"",
    "if grep -qE 'ratatui|crossterm' \"$trees/app.txt\"",
    'cargo tree -p cicada -e normal > "$trees/none.txt"',
    'cargo tree -p cicada -e normal --features sqlite > "$trees/sqlite.txt"',
    'cargo tree -p cicada -e normal --features postgres > "$trees/postgres.txt"',
    "cargo tree --workspace --target all -f '{p} [{f}]' --prefix none > \"$trees/workspace.txt\"",
    'grep -q sqlx-sqlite "$trees/sqlite.txt"',
    'grep -q sqlx-postgres "$trees/postgres.txt"',
    'if grep -q sqlx "$trees/none.txt"',
    'if grep -q sqlx-postgres "$trees/sqlite.txt"',
    "if grep -qE 'sqlx-sqlite|libsqlite3-sys' \"$trees/postgres.txt\"",
    "if grep -qE 'rustls|native-tls' \"$trees/sqlite.txt\"",
    "awk '/^rustls v/ && /[[,]ring[],]/ && /[[,]aws_lc_rs[],]/ { two = 1 } END { exit !two }' \"$trees/workspace.txt\"",
]


def full_text() -> str:
    """The full policy's commands, with the scripts it runs inlined."""
    lines = []
    for command in REAL_POLICY["kinds"]["full"]:
        lines.append(" ".join(command["argv"]))
        if command["argv"][0] == "bash":
            lines.append((ROOT / command["argv"][1]).read_text())
    return "\n".join(lines)


class PolicyTests(unittest.TestCase):
    def test_the_real_policy_parses(self):
        gate.parse_policy((SCRIPTS / "gate-policy.json").read_bytes())

    def test_unsupported_or_malformed_policies_are_refused(self):
        cases = {
            "schema": dict(policy(), schema=2),
            "kind": {**policy(), "kinds": {"full": policy()["kinds"]["full"]}},
            "argv": policy(full=[{"name": "x", "argv": "cargo test"}]),
            "names": policy(full=[{"name": "x", "argv": ["a"]}, {"name": "x", "argv": ["b"]}]),
            "allowlist": {**policy(), "docs_allowlist": {"files": []}},
            "pg": {k: v for k, v in policy().items() if k != "pg_url_variable"},
        }
        for name, value in cases.items():
            with self.subTest(name), self.assertRaises(gate.GateError):
                gate.parse_policy(json.dumps(value).encode())
        with self.assertRaises(gate.GateError):
            gate.parse_policy(b"not json")

    def test_every_former_ci_check_stays_in_the_full_policy(self):
        text = full_text()
        for line in FORMER_CI:
            with self.subTest(line):
                self.assertIn(line, text)
        python = REAL_POLICY["kinds"]["full"][0]["argv"]
        self.assertEqual(python[:6], ["python3", "-B", "-m", "unittest", "discover", "-s"])
        self.assertTrue(fnmatch("test_docs_maintenance.py", python[-1]), "the former Python test is discovered")
        features = (ROOT / "scripts/ci/feature-builds.sh").read_text()
        self.assertIn('export RUSTFLAGS="-D warnings"', features)
        self.assertEqual(REAL_POLICY["env"]["CARGO_PROFILE_DEV_DEBUG"], "line-tables-only")
        self.assertEqual(REAL_POLICY["pg_url_variable"], "CICADA_PG_URL")

    def test_docs_and_e2e_policies(self):
        docs = [" ".join(c["argv"]) for c in REAL_POLICY["kinds"]["docs"]]
        self.assertIn("cargo test -p cicada-examples --doc", docs)
        self.assertIn("cargo test -p cicada-examples --test docs", docs)
        self.assertIn("cargo test -p cicada-examples --test skill", docs)
        self.assertTrue(docs[0].startswith("python3 -B -m unittest discover -s scripts"))
        (e2e,) = REAL_POLICY["kinds"]["e2e"]
        self.assertEqual(e2e["argv"], ["cargo", "test", "-p", "cicada-e2e", "--", "--nocapture"])
        self.assertEqual(e2e["env"], {"CICADA_E2E": "1"}, "no filter")

    def test_ci_scripts_keep_shell_error_propagation(self):
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "cargo"
            fake.write_text("#!/bin/sh\necho fake cargo fails >&2\nexit 3\n")
            fake.chmod(0o755)
            env = {"PATH": f"{temp}{os.pathsep}{os.environ['PATH']}", "TMPDIR": temp}
            for script in ("feature-builds.sh", "dependency-trees.sh"):
                with self.subTest(script):
                    text = (ROOT / "scripts/ci" / script).read_text()
                    self.assertIn("set -euo pipefail", text)
                    done = subprocess.run(["bash", str(ROOT / "scripts/ci" / script)], cwd=temp, env=env,
                                          capture_output=True, text=True)
                    self.assertNotEqual(done.returncode, 0)

    def test_client_tests_need_node_26_and_never_skip(self):
        full = [c["argv"] for c in REAL_POLICY["kinds"]["full"]]
        self.assertIn(["bash", "scripts/ci/client-tests.sh"], full)
        script = ROOT / "scripts/ci/client-tests.sh"
        self.assertIn("set -euo pipefail", script.read_text())
        self.assertIn("node --test examples/tests/websocket_session_client.test.mjs", script.read_text())
        with tempfile.TemporaryDirectory() as temp:
            bash = subprocess.run(["sh", "-c", "command -v bash"], capture_output=True, text=True).stdout.strip()
            bin_dir = Path(temp) / "bin"
            bin_dir.mkdir()
            for tool in ("bash", "env"):
                found = subprocess.run(["sh", "-c", f"command -v {tool}"], capture_output=True, text=True)
                (bin_dir / tool).symlink_to(found.stdout.strip())
            env = {"PATH": str(bin_dir), "TMPDIR": temp}
            missing = subprocess.run([bash, str(script)], cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn("node not found on PATH; install Node 26 or later", missing.stderr)
            node = bin_dir / "node"
            node.write_text('#!/bin/sh\ncase "$1" in -p) echo 24;; --version) echo v24.22.1;; *) exit 0;; esac\n')
            node.chmod(0o755)
            old = subprocess.run([bash, str(script)], cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertNotEqual(old.returncode, 0, "an old node fails, it does not pass by running nothing")
            self.assertIn("node v24.22.1 is older than 26", old.stderr)

    def test_advisories_are_checked_and_each_accepted_one_has_a_reason(self):
        full = [c["argv"] for c in REAL_POLICY["kinds"]["full"]]
        self.assertIn(["cargo", "deny", "check", "advisories"], full)
        self.assertNotIn("--disable-fetch", full_text(), "a stale database must not pass")
        deny = tomllib.loads((ROOT / "deny.toml").read_text())
        self.assertEqual(list(deny), ["advisories"])
        ignored = deny["advisories"]["ignore"]
        self.assertEqual([entry["id"] for entry in ignored], ["RUSTSEC-2026-0543"])
        self.assertTrue(all(entry["reason"].strip() for entry in ignored))


class EnvironmentTests(unittest.TestCase):
    AMBIENT = {
        "CICADA_UPDATE_CONTRACTS": "true", "CICADA_UPDATE_CLI": "true", "CICADA_UPDATE_OPENAPI": "true",
        "CICADA_E2E_ONLY": "NodeFaultTest", "PROPTEST_CASES": "1", "RUSTFLAGS": "-A warnings",
        "CARGO_ENCODED_RUSTFLAGS": "x", "RUSTDOCFLAGS": "x", "RUSTC_WRAPPER": "sccache", "CARGO_TARGET_DIR": "/elsewhere",
        "CICADA_PG_URL": "postgres://ambient", "CICADA_E2E": "1", "GIT_DIR": "/x", "PATH": "/bin", "HOME": "/home/me",
        "LC_ALL": "C", "SSH_AUTH_SOCK": "/agent", "DOCKER_HOST": "unix:///docker.sock", "RUSTUP_TOOLCHAIN": "stable",
    }

    def test_command_environment_drops_overrides_and_sets_its_own(self):
        with mock.patch.dict(os.environ, self.AMBIENT, clear=True):
            env = gate.command_env(REAL_POLICY, "postgres://gate:secret@db:5432/cicada")
        for name in ("CICADA_UPDATE_CONTRACTS", "CICADA_UPDATE_CLI", "CICADA_UPDATE_OPENAPI", "CICADA_E2E_ONLY",
                     "PROPTEST_CASES", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC_WRAPPER",
                     "CARGO_TARGET_DIR", "CICADA_E2E", "GIT_DIR"):
            self.assertNotIn(name, env)
        for name in ("PATH", "HOME", "LC_ALL", "SSH_AUTH_SOCK", "DOCKER_HOST", "RUSTUP_TOOLCHAIN"):
            self.assertEqual(env[name], self.AMBIENT[name])
        self.assertEqual(env["CICADA_PG_URL"], "postgres://gate:secret@db:5432/cicada")
        self.assertEqual(env["CARGO_PROFILE_DEV_DEBUG"], "line-tables-only")
        self.assertEqual(env["PYTHONDONTWRITEBYTECODE"], "1")

    def test_pg_url_must_be_set_and_its_metadata_has_no_credentials(self):
        with mock.patch.dict(os.environ, {"CICADA_GATE_PG_URL": " "}):
            with self.assertRaises(gate.GateError):
                gate.pg_url()
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(gate.pg_url(), gate.DEFAULT_PG_URL)
        meta = gate.pg_metadata("postgres://user:hunter2@db.local:6543/cicada?sslpassword=hunter3")
        self.assertEqual(meta, {"host": "db.local", "port": 6543, "database": "cicada"})
        self.assertNotIn("hunter", json.dumps(meta))

    def test_checks_runs_the_policy_here_and_stops_at_a_failure(self):
        sandbox = Sandbox(self)
        out = sandbox.dir / "seen.json"
        dump = f"import json, os; json.dump(dict(os.environ), open({str(out)!r}, 'w'))"
        sandbox.write("scripts/gate-policy.json", json.dumps(policy(
            full=[{"name": "dump", "argv": ["python3", "-c", dump]},
                  {"name": "fails", "argv": ["python3", "-c", "import sys; sys.exit(4)"]},
                  {"name": "never", "argv": ["python3", "-c", f"open({str(sandbox.dir / 'never')!r}, 'w')"]}])))
        ambient = {"CICADA_GATE_PG_URL": "postgres://bad:pw@localhost:1/none", "CICADA_E2E_ONLY": "X",
                   "RUSTFLAGS": "-A warnings"}
        done = sandbox.gate("checks", "--scope", "full", env=ambient)
        self.assertEqual(done.returncode, 4, done.stderr)
        seen = json.loads(out.read_text())
        self.assertEqual(seen["CICADA_PG_URL"], "postgres://bad:pw@localhost:1/none", "a bad URL reaches the tests")
        self.assertNotIn("CICADA_E2E_ONLY", seen)
        self.assertNotIn("RUSTFLAGS", seen)
        self.assertFalse((sandbox.dir / "never").exists())
        self.assertFalse(sandbox.state.exists(), "checks records nothing")
        self.assertEqual(sandbox.gate("checks", "--scope", "full", env={"CICADA_GATE_PG_URL": ""}).returncode, 1)


class WorkflowTests(unittest.TestCase):
    def test_only_a_manual_ci_workflow_remains(self):
        workflows = sorted(p.name for p in (ROOT / ".github/workflows").iterdir())
        self.assertEqual(workflows, ["ci.yml"])
        text = (ROOT / ".github/workflows/ci.yml").read_text()
        on = text.split("\non:\n", 1)[1].split("\n\n", 1)[0]
        self.assertEqual([line.strip() for line in on.splitlines() if line.strip()], ["workflow_dispatch:"])
        for trigger in ("push:", "pull_request:", "workflow_call:", "tags:"):
            self.assertNotIn(trigger, text)
        self.assertIn("python3 scripts/gate.py checks --scope full", text)
        install = "cargo install cargo-deny --locked --version 0.20.2"
        self.assertLess(text.index(install), text.index("python3 scripts/gate.py checks --scope full"))
        self.assertIn("CICADA_GATE_PG_URL: postgres://cicada:cicada@localhost:55432/cicada", text)
        self.assertNotIn("contents: write", text)


if __name__ == "__main__":
    unittest.main()
