"""Grok (xAI) API-driven MCP tests.

Research §1.6: POST https://api.x.ai/v1/responses with the MCP server declared
in tools[] (server_url, server_label required; authorization carries the dev
bearer; require_approval/connector_id NOT supported in this variant).
Primary, scriptable, nightly-capable. Model pinned to grok-4.7 (docs publish
no supported-model list; noted as UNVERIFIED gap).

The MCP server_url must be publicly reachable — api.x.ai cannot dial
localhost. setup refuses a loopback URL with a clear message (use the dev
cloud host or a tunnel).
"""
from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request

from .. import config as config_mod
from .cli_base import CLIProvider, ConfigError, EnvError

RESPONSES_URL = "https://api.x.ai/v1/responses"
MODEL = "grok-4.7"

# dynamic_credentials lives in the product skill-creator skill; it is the
# single source of truth for the authd surrogate exchange. The harness
# imports it (like the openrouter skill does) rather than vendoring the
# authd protocol.
_DYNAMIC_CREDS_PATH = "/opt/hatch/skills/skill-creator/bin"


class GrokProvider(CLIProvider):
    surface = "api"
    name = "grok"
    required_env = ("XAI_API_KEY",)
    binary = None  # pure HTTPS; no binary

    # -- key resolution -------------------------------------------------
    def _vault_surrogate(self) -> str | None:
        """Surrogate for custom.xai from the Secure Vault, or None.

        The surrogate is sent literally as the bearer; the egress layer
        exchanges it for the real key. Never logged, never persisted.
        """
        try:
            if _DYNAMIC_CREDS_PATH not in sys.path:
                sys.path.insert(0, _DYNAMIC_CREDS_PATH)
            from dynamic_credentials import dynamic_credential_entry
            entry = dynamic_credential_entry("custom.xai", "access_token")
            return str(entry["surrogate"]).strip() or None
        except Exception:
            return None

    def _auth_header(self) -> str:
        """Authorization header: explicit env key wins, vault is fallback."""
        env_key = os.environ.get("XAI_API_KEY")
        if env_key:
            return "Bearer " + env_key
        surr = self._vault_surrogate()
        if surr:
            return "Bearer " + surr
        raise EnvError("XAI_API_KEY")

    def check_env(self) -> None:
        if os.environ.get("XAI_API_KEY") or self._vault_surrogate():
            return
        raise EnvError("XAI_API_KEY")

    def _mcp_tool(self) -> dict:
        cfg = self.cfg["server"]
        tool: dict = {
            "type": "mcp",
            "server_label": self.pcfg.get("server_label", "maidan"),
            "server_url": config_mod.mcp_url(self.cfg),
        }
        desc = self.pcfg.get("server_description")
        if desc:
            tool["server_description"] = desc
        # Dev bearer rides in the MCP Authorization header; no OAuth flow.
        bearer = os.environ.get("MAIDAN_DEV_BEARER")
        if bearer:
            tool["authorization"] = bearer
        return tool

    def _public_url_or_die(self) -> None:
        url = config_mod.mcp_url(self.cfg)
        host = urllib.request.urlparse(url).hostname or ""
        if host in ("127.0.0.1", "localhost", "::1"):
            raise ConfigError(
                f"api.x.ai cannot reach the loopback MCP URL {url}; "
                "point the dev server at a public host or tunnel "
                "(set server.base_url in config.yaml) and retry")

    # -- lifecycle ------------------------------------------------------
    def install(self) -> None:
        """No install step: API-driven. Verifies key + URL reachability."""
        self.check_env()
        self._public_url_or_die()
        # Cheap validation: the models endpoint is free and key-gated.
        req = urllib.request.Request(
            "https://api.x.ai/v1/models",
            headers={"Authorization": self._auth_header()})
        try:
            with urllib.request.urlopen(req, timeout=20) as r:
                if r.status >= 400:
                    raise ConfigError(f"xAI key rejected (HTTP {r.status})")
        except urllib.error.HTTPError as e:
            # The key authenticates but the account can't call (e.g. the
            # team's credits are spent): surface xAI's own message, not a
            # traceback and not KEY_MISSING.
            try:
                body = e.read()[:300].decode(errors="replace")
            except Exception:
                body = ""
            raise ConfigError(
                f"xAI API refused the request (HTTP {e.code}): {body}")
        print("OK: grok XAI_API_KEY valid; MCP URL is public")

    def is_setup(self) -> bool:
        try:
            self.check_env()
        except EnvError:
            return False
        try:
            self._public_url_or_die()
        except ConfigError:
            return False
        return True

    def run_prompt(self, prompt: str, timeout: int = 180) -> str:
        """POST /v1/responses with the MCP server declared; return text."""
        self.check_env()
        self._public_url_or_die()
        body = {
            "model": self.pcfg.get("model", MODEL),
            "input": prompt,
            "tools": [self._mcp_tool()],
        }
        req = urllib.request.Request(
            RESPONSES_URL,
            data=json.dumps(body).encode(),
            headers={
                "Authorization": self._auth_header(),
                "Content-Type": "application/json",
            },
            method="POST")
        with urllib.request.urlopen(req, timeout=timeout) as r:
            payload = json.load(r)
        return self._extract_text(payload)

    @staticmethod
    def _extract_text(payload: dict) -> str:
        """Defensive: server-side probe is the primary assertion; this just
        needs the assistant's text for the secondary UI check."""
        parts: list[str] = []
        for item in payload.get("output", []) or []:
            itype = item.get("type", "")
            if itype == "message":
                for c in item.get("content", []) or []:
                    if c.get("type") == "output_text":
                        parts.append(c.get("text", ""))
            elif itype in ("mcp_call", "mcp_approval_request"):
                # Tool-call items: note their presence; the probe asserts.
                parts.append(f"[{itype}:{item.get('server_label', '')}]")
        return "\n".join(parts).strip()

    def build_request_body(self, prompt: str) -> dict:
        """Exposed for unit tests (no network, no key)."""
        return {
            "model": self.pcfg.get("model", MODEL),
            "input": prompt,
            "tools": [self._mcp_tool()],
        }
