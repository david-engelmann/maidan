"""Gemini CLI provider: extension-based MCP wiring.

Research §1.4: author gemini-extension.json (mcpServers) at the extension
root; local dev loop is `gemini extensions link .` (symlink — edits live).
Extension gallery install is the distribution path; not used here.

Gemini CLI does NOT implement elicitation/create — no elicitation tests.
"""
from __future__ import annotations

import json
import os
import shutil

from .. import config as config_mod
from .cli_base import CLIProvider, BinaryError

EXTENSION_DIR_NAME = "maidan"


class GeminiProvider(CLIProvider):
    surface = "cli"
    name = "gemini"
    # Non-interactive runs require the API key (Google-login cached creds
    # don't exist on the headless harness VM).
    required_env = ("GEMINI_API_KEY",)
    binary = "gemini"
    npm_package = "@google/gemini-cli"

    def _extension_dir(self) -> str:
        root = os.path.dirname(os.path.dirname(
            os.path.abspath(config_mod.DEFAULT_CONFIG)))
        return os.path.join(root, "extensions", EXTENSION_DIR_NAME)

    def _sync_extension_url(self) -> None:
        """Rewrite the extension's MCP URL to the configured server."""
        path = os.path.join(self._extension_dir(), "gemini-extension.json")
        with open(path) as f:
            doc = json.load(f)
        url = config_mod.mcp_url(self.cfg)
        servers = doc.get("mcpServers", {})
        if "maidan" not in servers:
            servers["maidan"] = {}
        # Key name verified against the installed CLI's schema (see README).
        servers["maidan"]["httpUrl"] = url
        doc["mcpServers"] = servers
        with open(path, "w") as f:
            json.dump(doc, f, indent=2)

    # -- lifecycle ------------------------------------------------------
    def install(self) -> None:
        """Link the local extension (idempotent; edits reflect immediately)."""
        self.ensure_binary()
        self._sync_extension_url()
        # --consent skips the interactive security prompt (dev only).
        r = self.run(["gemini", "extensions", "link", self._extension_dir(),
                      "--consent"], timeout=60)
        if r.returncode != 0:
            raise RuntimeError(f"gemini extensions link failed: {r.stderr[:500]}")
        print(f"OK: gemini extension linked -> {self._extension_dir()}")

    def is_setup(self) -> bool:
        try:
            self.ensure_binary()
        except BinaryError:
            return False
        r = self.run(["gemini", "extensions", "list"], timeout=30)
        if r.returncode != 0:
            return False
        out = (r.stdout or "").lower()
        return "maidan" in out

    def run_prompt(self, prompt: str, timeout: int = 180) -> str:
        """`gemini -p`: non-interactive prompt; stdout is the response."""
        self.ensure_binary()
        r = self.run(["gemini", "-p", prompt], timeout=timeout)
        if r.returncode != 0:
            raise RuntimeError(
                f"gemini -p failed (exit {r.returncode}): {r.stderr[:500]}")
        return (r.stdout or "").strip()
