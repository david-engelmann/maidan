#!/usr/bin/env python3
"""Gates for mcp-sdk-clients arg bounds and docs wrangler cost contracts.

Run: PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.test_mcp_sdk_clients_gates
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TS = ROOT / "scripts/mcp-sdk-clients/typescript/client.mjs"
PY_CLIENT = ROOT / "scripts/mcp-sdk-clients/python/client.py"
SH = ROOT / "scripts/mcp-sdk-clients.sh"
COST = ROOT / "deploy/cloudflare/test/cost.test.js"
DOCS = ROOT / "deploy/cloudflare/wrangler.docs.jsonc"


def _run(cmd: list[str], env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=20,
        env={**os.environ, **(env or {})},
    )


class McpSdkClientGates(unittest.TestCase):
    def test_typescript_refuses_cleartext_off_loopback_before_sdk_import(self) -> None:
        r = _run(["node", str(TS), "http://example.com", "/mcp", "token-token"])
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("loopback", r.stderr)

    def test_typescript_refuses_bad_endpoint(self) -> None:
        r = _run(["node", str(TS), "http://127.0.0.1:9", "/admin", "token-token"])
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("/mcp", r.stderr)

    def test_typescript_refuses_credentialed_url(self) -> None:
        r = _run(["node", str(TS), "https://user:pass@maidan.world", "/mcp", "token-token"])
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("credentials", r.stderr)

    def test_python_refuses_cleartext_off_loopback_before_sdk_import(self) -> None:
        r = _run(["python3", str(PY_CLIENT), "http://example.com", "/mcp", "token-token"])
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("loopback", r.stderr)

    def test_python_refuses_bad_endpoint(self) -> None:
        r = _run(["python3", str(PY_CLIENT), "http://127.0.0.1:9", "/mcp/../admin", "token-token"])
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("/mcp", r.stderr)

    def test_python_assert_client_args_unit(self) -> None:
        import importlib.util

        spec = importlib.util.spec_from_file_location("mcp_sdk_py_client", PY_CLIENT)
        assert spec and spec.loader
        mod = importlib.util.module_from_spec(spec)
        # Loading executes only defs + main guard; assert_client_args is available.
        spec.loader.exec_module(mod)
        with self.assertRaises(SystemExit):
            mod.assert_client_args("http://evil.example", "/mcp", "token-token")
        mod.assert_client_args("http://127.0.0.1:18091", "/mcp/streamable", "token-token")

    def test_shell_refuses_out_of_range_port(self) -> None:
        # Source only the early checks by running bash -c against a copied head.
        # Full script would cargo-build; we assert the guard text and dry-run via env.
        text = SH.read_text()
        self.assertIn("refusing out-of-range port", text)
        self.assertIn("unknown client:", text)
        script = """
set -euo pipefail
port="${MAIDAN_MCP_PORT:-18091}"
if [[ ! "$port" =~ ^[0-9]+$ ]] || (( port < 1 || port > 65535 )); then
  echo "mcp-sdk-clients: refusing out-of-range port: $port" >&2
  exit 1
fi
clients=("$@")
[[ ${#clients[@]} -gt 0 ]] || clients=(typescript python)
for client in "${clients[@]}"; do
  case "$client" in
    typescript|python) ;;
    *) echo "mcp-sdk-clients: unknown client: ${client} (typescript or python)" >&2; exit 2 ;;
  esac
done
echo ok
"""
        r = _run(["bash", "-c", script, "--", "nope"], env={"MAIDAN_MCP_PORT": "99999"})
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("out-of-range port", r.stderr)
        r2 = _run(["bash", "-c", script, "bash", "ruby"])
        self.assertEqual(r2.returncode, 2)
        self.assertIn("unknown client", r2.stderr)
        r3 = _run(["bash", "-c", script, "bash", "typescript"], env={"MAIDAN_MCP_PORT": "18091"})
        self.assertEqual(r3.returncode, 0, r3.stderr)

    def test_docs_wrangler_has_no_container_or_cron(self) -> None:
        text = DOCS.read_text()
        self.assertNotIn('"containers"', text)
        self.assertNotIn('"crons"', text)
        self.assertIn("docs.maidan.world", text)
        cost = COST.read_text()
        self.assertIn("wrangler.docs.jsonc", cost)
        self.assertIn("no container", cost)

    def test_cost_test_passes(self) -> None:
        r = _run(["node", "--test", str(COST)])
        self.assertEqual(r.returncode, 0, r.stderr + r.stdout)


if __name__ == "__main__":
    unittest.main()
