"""GitHub Copilot CLI: TWO separate mechanisms (research §1.5 — the v1 doc
conflated them; the test plan covers both and never assumes one carries
the other).

(a) copilot-mcp: the Maidan MCP server added via `copilot mcp add`.
    The primary tool-access path.
(b) copilot-plugin: the Maidan plugin (skills packaging) via the
    marketplace dir-source flow (live reload on /restart — no reinstall).

Elicitation: the CLI supports form mode, but harness policy scopes
elicitation tests to Claude Code only. No elicitation tests here.
"""
from __future__ import annotations

import json
import os

from .. import config as config_mod
from .cli_base import CLIProvider, BinaryError

PLUGIN_DIR_NAME = "maidan"
MCP_NAME = "maidan"


class CopilotBase(CLIProvider):
    surface = "cli"
    binary = "copilot"
    npm_package = "@github/copilot"
    # Headless auth: fine-grained PAT v2 with "Copilot Requests" permission.
    # Classic PATs are NOT supported (official docs). Precedence order is the
    # CLI's own: COPILOT_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN.
    required_env_any = ("COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN")

    def _mcp_url(self) -> str:
        return config_mod.mcp_url(self.cfg)


class CopilotMCPProvider(CopilotBase):
    """Mechanism (a): MCP server via `copilot mcp add`."""
    name = "copilot-mcp"

    def install(self) -> None:
        self.ensure_binary()
        # Remove-then-add keeps install idempotent across URL changes.
        # Syntax verified against copilot 1.0.92.
        self.run(["copilot", "mcp", "remove", MCP_NAME], timeout=30)
        r = self.run(["copilot", "mcp", "add", "--transport", "http",
                      MCP_NAME, self._mcp_url()], timeout=60)
        if r.returncode != 0:
            raise RuntimeError(
                f"copilot mcp add failed: {(r.stderr or r.stdout)[:500]}")
        print(f"OK: copilot MCP server '{MCP_NAME}' -> {self._mcp_url()}")

    def is_setup(self) -> bool:
        try:
            self.ensure_binary()
        except BinaryError:
            return False
        r = self.run(["copilot", "mcp", "list"], timeout=30)
        if r.returncode != 0:
            return False
        out = (r.stdout or "")
        return MCP_NAME in out and self._mcp_url() in out

    def run_prompt(self, prompt: str, timeout: int = 180) -> str:
        self.ensure_binary()
        r = self.run(["copilot", "-p", prompt], timeout=timeout)
        if r.returncode != 0:
            raise RuntimeError(
                f"copilot -p failed (exit {r.returncode}): {r.stderr[:500]}")
        return (r.stdout or "").strip()


class CopilotPluginProvider(CopilotBase):
    """Mechanism (b): plugin via marketplace dir-source flow.

    Research §1.5: plugins from a local directory-source marketplace load
    live from their real directory and pick up edits on /restart — no
    reinstall needed (the stale-cache quirk applies to direct installs).
    """
    name = "copilot-plugin"

    def _plugin_dir(self) -> str:
        root = os.path.dirname(os.path.dirname(
            os.path.abspath(config_mod.DEFAULT_CONFIG)))
        return os.path.join(root, "plugins", PLUGIN_DIR_NAME)

    def _marketplace_dir(self) -> str:
        root = os.path.dirname(os.path.dirname(
            os.path.abspath(config_mod.DEFAULT_CONFIG)))
        return os.path.join(root, "plugins")

    def install(self) -> None:
        self.ensure_binary()
        # Marketplace dir-source: verified against copilot 1.0.92.
        # marketplace.json needs owner.name; plugins load live from the
        # directory (edits take effect next session, nothing copied).
        mdir = self._marketplace_dir()
        r = self.run(["copilot", "plugin", "marketplace", "add", mdir],
                     timeout=60)
        if r.returncode != 0 and "already" not in (r.stderr or "").lower():
            raise RuntimeError(
                f"copilot plugin marketplace add failed: "
                f"{(r.stderr or r.stdout)[:500]}")
        r = self.run(["copilot", "plugin", "install", "maidan@maidan-dev"],
                     timeout=60)
        if r.returncode != 0:
            raise RuntimeError(
                f"copilot plugin install failed: {(r.stderr or r.stdout)[:500]}")
        print(f"OK: copilot plugin maidan@maidan-dev -> {self._plugin_dir()}")

    def is_setup(self) -> bool:
        try:
            self.ensure_binary()
        except BinaryError:
            return False
        r = self.run(["copilot", "plugin", "list"], timeout=30)
        if r.returncode != 0:
            return False
        return PLUGIN_DIR_NAME in (r.stdout or "")

    def run_prompt(self, prompt: str, timeout: int = 180) -> str:
        self.ensure_binary()
        r = self.run(["copilot", "-p", prompt], timeout=timeout)
        if r.returncode != 0:
            raise RuntimeError(
                f"copilot -p failed (exit {r.returncode}): {r.stderr[:500]}")
        return (r.stdout or "").strip()
