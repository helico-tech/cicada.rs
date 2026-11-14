#!/usr/bin/env python3
"""Capture the real TUI against an isolated SQLite sample node; write ANSI for a terminal renderer."""

import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def until(read, ready, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = read()
        if ready(result):
            return result
        time.sleep(0.05)
    raise RuntimeError("The sample did not reach the expected state before capture")


def capture(output):
    sample = ROOT / "target/debug/cicada-sample-service"
    operator = ROOT / "target/debug/cicada"
    if not sample.exists() or not operator.exists():
        raise SystemExit("Build sample-service and cicada-app with tui first; see docs/assets/README.md")
    # No user configuration, credentials or production store enter this capture.
    env = {key: value for key, value in os.environ.items() if not key.startswith(("CICADA_", "OTEL_"))}
    for key in ("API_TOKEN", "DATABASE_URL", "PORT", "SQLITE_PATH", "NODE_ID"):
        env.pop(key, None)
    env.update(TERM="xterm-256color", COLORTERM="truecolor", CICADA_NODE_ID="docs-demo", NO_COLOR="1")
    with tempfile.TemporaryDirectory(prefix="cicada-tui-") as temporary:
        temp = Path(temporary)
        config = temp / "config.toml"
        env["CICADA_CONFIG"] = str(config)
        log = temp / "node.log"
        with log.open("w") as stream:
            node = subprocess.Popen(
                [str(sample), "serve", "--store", str(temp / "actors.db"), "--listen", "127.0.0.1:0",
                 "--types", "sync,reports,cicada.dead-letters"],
                env=env, stdout=stream, stderr=subprocess.STDOUT,
            )
        child = None
        master = None
        try:
            match = until(lambda: re.search(r"Listening on (http://[^\s\x1b]+)", log.read_text()), bool)
            server = match.group(1)
            # Only ephemeral loopback configuration is written, inside the temporary directory.
            config.write_text(f'current-context = "demo"\n[contexts.demo]\nserver = "{server}"\n')

            def request(method, path, body=None):
                data = None if body is None else json.dumps(body).encode()
                req = urllib.request.Request(server + path, data=data, method=method,
                                             headers={"Content-Type": "application/json"})
                with urllib.request.urlopen(req, timeout=5) as response:
                    text = response.read()
                    return json.loads(text) if text else None

            for key in ("acme", "contoso"):
                request("PUT", f"/actors/reports/{key}", {"state": {"runs": []}})
                request("PUT", f"/actors/sync/{key}",
                        {"state": {"source": f"https://{key}.example.test/items", "imported": 0}})
                request("POST", f"/actors/sync/{key}/messages", {"id": f"sync-{key}", "type": "Run"})
                until(lambda: request("GET", f"/actors/sync/{key}"), lambda actor: actor["state"]["imported"] == 25)
            request("POST", "/actors/sync/acme/mailbox/pause")
            request("POST", "/actors/sync/acme/messages",
                    {"id": "manual-refresh", "type": "Run", "delay": "PT5M"})
            request("POST", "/actors/sync/acme/messages",
                    {"id": "weekly-audit", "type": "Run", "delay": "P1D"})

            cols, rows = 140, 28
            child, master = pty.fork()
            if child == 0:
                fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
                env.pop("NO_COLOR", None)
                os.execve(str(operator), [str(operator), "--context", "demo", "tui"], env)
            chunks = []

            def read_for(seconds):
                deadline = time.monotonic() + seconds
                while time.monotonic() < deadline:
                    ready, _, _ = select.select([master], [], [], min(0.1, max(0, deadline - time.monotonic())))
                    if ready:
                        try:
                            chunk = os.read(master, 65536)
                        except OSError as error:
                            if error.errno == errno.EIO:
                                break
                            raise
                        if not chunk:
                            break
                        chunks.append(chunk)
                        # Respond to terminal probes without changing the app's output.
                        if b"\x1b[6n" in chunk:
                            os.write(master, b"\x1b[1;1R")
            read_for(1.0)
            os.write(master, b":actor sync/acme\r")
            read_for(2.5)
            ansi = b"".join(chunks).decode("utf-8")
            if "sync/acme" not in ansi or "manual-refresh" not in ansi:
                raise RuntimeError("The TUI did not load the seeded actor and mailbox")
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps({"cols": cols, "rows": rows, "ansi": ansi}, ensure_ascii=False))
            print(f"Captured live TUI to {output}")
        finally:
            if child:
                try:
                    os.kill(child, signal.SIGTERM)
                    deadline = time.monotonic() + 2
                    while os.waitpid(child, os.WNOHANG) == (0, 0):
                        if time.monotonic() >= deadline:
                            os.kill(child, signal.SIGKILL)
                            os.waitpid(child, 0)
                            break
                        time.sleep(0.05)
                except ProcessLookupError:
                    pass
            if master is not None:
                os.close(master)
            node.send_signal(signal.SIGTERM)
            try:
                node.wait(timeout=10)
            except subprocess.TimeoutExpired:
                node.kill()
                node.wait()
                raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="Temporary ANSI JSON, rendered by render-docs.mjs")
    capture(parser.parse_args().output.resolve())
