"""Claude Code provider: `claude mcp add` + `claude -p`.

Research §1.8/§5.2: Claude Code is the ONLY in-scope provider implementing
elicitation/create (form + URL modes). The `elicitation` suite runs here and
only here — harness policy enforced by
tests/test_harness.py::test_elicitation_only_claude_code.

Elicitation test design: the prompt triggers Maidan's approval flow, which
elicits the decision. The client must answer the elicitation/create request;
auto-approval comes from the harness-managed settings file (dev only), never
from editing the server. Requires config elicitation.enabled=true; otherwise
the suite is skipped with a reason (never failed, never faked).
"""
from __future__ import annotations

from .. import config as config_mod
from .cli_base import CLIProvider, BinaryError

MCP_NAME = "maidan"


class ClaudeCodeProvider(CLIProvider):
    surface = "cli"
    name = "claude-code"
    # Non-interactive runs require the API key (no cached login on the
    # headless harness VM).
    required_env = ("ANTHROPIC_API_KEY",)
    binary = "claude"
    npm_package = "@anthropic-ai/claude-code"
    supports_elicitation = True

    def _mcp_url(self) -> str:
        return config_mod.mcp_url(self.cfg)

    def _base_argv(self) -> list[str]:
        # Dev-only unattended operation: skip interactive permission prompts.
        # Never use outside the harness.
        argv = ["claude", "-p"]
        if self.pcfg.get("skip_permissions", True):
            argv.append("--dangerously-skip-permissions")
        return argv

    # -- lifecycle ------------------------------------------------------
    def install(self) -> None:
        self.ensure_binary()
        # Remove-then-add keeps install idempotent across URL changes.
        # --scope user: global config, independent of cwd (default is local).
        self.run(["claude", "mcp", "remove", MCP_NAME, "--scope", "user"],
                 timeout=30)
        r = self.run(["claude", "mcp", "add", "--transport", "http",
                      "--scope", "user", MCP_NAME, self._mcp_url()], timeout=60)
        if r.returncode != 0:
            raise RuntimeError(
                f"claude mcp add failed: {(r.stderr or r.stdout)[:500]}")
        print(f"OK: claude-code MCP '{MCP_NAME}' -> {self._mcp_url()}")

    def is_setup(self) -> bool:
        try:
            self.ensure_binary()
        except BinaryError:
            return False
        r = self.run(["claude", "mcp", "list"], timeout=30)
        if r.returncode != 0:
            return False
        out = (r.stdout or "")
        return MCP_NAME in out and self._mcp_url() in out

    def run_prompt(self, prompt: str, timeout: int = 180) -> str:
        self.ensure_binary()
        r = self.run(self._base_argv() + [prompt], timeout=timeout)
        if r.returncode != 0:
            raise RuntimeError(
                f"claude -p failed (exit {r.returncode}): {r.stderr[:500]}")
        return (r.stdout or "").strip()
