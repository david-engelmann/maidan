"""Cursor provider: config-file setup only — NO unattended runs.

Research §1.3: Cursor reads ~/.cursor/mcp.json (global) or .cursor/mcp.json
(project-scoped). Iteration is config-edit + reload — fastest of the UI
providers. BUT: Cursor does not reliably refresh stored OAuth tokens; an
expired server flips to "Logged out" and needs manual re-auth. Consequence:
Cursor cannot sit in the unattended nightly without a human on call.

So: `harness setup cursor` writes the project-scoped config (the committable
dev setup), `harness test cursor` / nightly refuse with MANUAL_ONLY, and the
L3b checklist (checklists/cursor.md) carries the human flow.
"""
from __future__ import annotations

import json
import os

from .. import config as config_mod
from .cli_base import CLIProvider

PROJECT_DIR_NAME = "cursor-project"


class CursorProvider(CLIProvider):
    surface = "manual"
    name = "cursor"
    manual_only = True
    binary = None
    # Cursor supports elicitation form mode (research §5.2), but harness
    # policy scopes elicitation tests to Claude Code only.

    def _project_dir(self) -> str:
        root = os.path.dirname(os.path.dirname(
            os.path.abspath(config_mod.DEFAULT_CONFIG)))
        return os.path.join(root, PROJECT_DIR_NAME)

    def _config_path(self) -> str:
        return os.path.join(self._project_dir(), ".cursor", "mcp.json")

    def install(self) -> None:
        """Write the project-scoped MCP config (idempotent)."""
        path = self._config_path()
        os.makedirs(os.path.dirname(path), exist_ok=True)
        doc: dict = {}
        if os.path.exists(path):
            with open(path) as f:
                try:
                    doc = json.load(f)
                except Exception:
                    doc = {}
        servers = doc.setdefault("mcpServers", {})
        servers["maidan"] = {"url": config_mod.mcp_url(self.cfg)}
        with open(path, "w") as f:
            json.dump(doc, f, indent=2)
        print(f"OK: cursor MCP config -> {path}")
        print("Open that folder in Cursor; Settings → MCP shows the server. "
              "First connect triggers the OAuth browser flow (manual).")

    def is_setup(self) -> bool:
        path = self._config_path()
        if not os.path.exists(path):
            return False
        try:
            with open(path) as f:
                doc = json.load(f)
        except Exception:
            return False
        srv = (doc.get("mcpServers") or {}).get("maidan") or {}
        return srv.get("url") == config_mod.mcp_url(self.cfg)

    def run_prompt(self, prompt: str, timeout: int = 180) -> str:
        raise NotImplementedError(
            "cursor cannot run unattended (token refresh is unreliable; "
            "research §1.3). Use checklists/cursor.md.")
