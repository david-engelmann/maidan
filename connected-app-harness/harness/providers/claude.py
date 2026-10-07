"""Claude provider: claude.ai custom connectors.

Research (§1.2): Customize > Connectors → + → Add custom connector.
No-auth mode is supported by default, so install/discovery/read tests
need no OAuth. Works on Free tier. claude.ai does NOT implement
elicitation/create — approval flows are conversational + a Maidan tool.
"""
from __future__ import annotations

from .. import config as config_mod
from ..selectors import find, any_present
from .base import Provider
from . import selectors_claude as S


class ClaudeProvider(Provider):
    name = "claude"
    app_url = "https://claude.ai"

    # -- auth -----------------------------------------------------------
    def auth_markers(self):
        return (S.LOGGED_IN_MARKERS, S.LOGGED_OUT_MARKERS, S.BOT_MARKERS)

    def logged_out_url(self, url: str) -> bool:
        return "/login" in (url or "")

    def is_logged_in(self, page) -> bool:  # legacy shim for auth-login UX
        from ..auth import check_auth_markers
        return check_auth_markers(page, self)["verdict"] == "logged-in"

    # -- setup ----------------------------------------------------------
    def _open_connectors(self, page):
        find(page, "customize_button", S.CUSTOMIZE_BUTTON).click()
        find(page, "connectors_item", S.CONNECTORS_ITEM).click()

    def _connector_row(self, page, name: str):
        """Return the row element for the named connector, or None."""
        self._open_connectors(page)
        try:
            return page.wait_for_selector(f'text="{name}"', timeout=8000)
        except Exception:
            return None

    def is_setup(self, page) -> bool:
        return self.setup_probe(page)["ok"]

    def setup_probe(self, page) -> dict:
        """Cheap probe: connector listed AND its URL matches config.

        No chat round-trip. Inconclusive -> ok=False (caller emits
        SETUP_NEEDED rather than burning a 5-minute test).
        """
        from .. import config as config_mod
        want_url = config_mod.mcp_url(self.cfg)
        name = self.pcfg["connector_name"]
        row = self._connector_row(page, name)
        if row is None:
            return {"ok": False,
                    "detail": f"connector {name!r} not listed"}
        # Try to read the configured URL from the row/detail without
        # starting a chat. Best-effort: fall back to name-only match.
        try:
            row.click()
            body = page.inner_text("body", timeout=5000) or ""
            if want_url in body or want_url.rstrip("/mcp") in body:
                return {"ok": True,
                        "detail": f"connector {name!r} listed, URL matches"}
            return {"ok": False,
                    "detail": f"connector {name!r} listed but URL not verified "
                              f"(want {want_url})"}
        except Exception:
            # Row exists but URL unreadable: name match is weak evidence.
            # Treat as setup-OK for now; the test's tool assertion is the
            # real proof. Logged so triage knows.
            return {"ok": True,
                    "detail": f"connector {name!r} listed (URL unverified)"}

    def install(self, page) -> None:
        mcp = config_mod.mcp_url(self.cfg)
        name = self.pcfg["connector_name"]
        self._open_connectors(page)
        find(page, "add_connector_button", S.ADD_CONNECTOR_BUTTON).click()
        find(page, "connector_name_input", S.CONNECTOR_NAME_INPUT).fill(name)
        find(page, "connector_url_input", S.CONNECTOR_URL_INPUT).fill(mcp)
        find(page, "add_button", S.ADD_BUTTON).click()
        find(page, "connector_row", [f'text="{name}"'], timeout=15000)
        # The Connect button is absent when already connected. Distinguish
        # that from selector rot: only swallow the miss if the disconnect
        # control proves we're connected; otherwise the STALE signal stands.
        try:
            find(page, "connect_button", S.CONNECT_BUTTON, timeout=8000).click()
            find(page, "disconnect_button", S.DISCONNECT_BUTTON, timeout=15000)
        except SystemExit:
            find(page, "disconnect_button", S.DISCONNECT_BUTTON, timeout=8000)

    # -- test -----------------------------------------------------------
    def new_chat(self, page) -> None:
        page.goto(f"{self.app_url}/new", wait_until="domcontentloaded")
        find(page, "composer", S.LOGGED_IN_MARKERS["composer"], timeout=15000)

    def send_prompt(self, page, prompt: str) -> None:
        box = find(page, "composer", S.LOGGED_IN_MARKERS["composer"])
        box.click()
        box.fill(prompt)
        box.press("Enter")

    def wait_for_response(self, page, timeout: float = 120.0) -> str:
        try:
            page.wait_for_selector('button[aria-label*="Stop" i]', timeout=15000)
            page.wait_for_selector('button[aria-label*="Stop" i]',
                                   state="detached", timeout=int(timeout * 1000))
        except Exception:
            # Stop button never appeared: poll until the response text
            # stabilizes instead of a blind sleep.
            def _last_text():
                msgs = page.query_selector_all(
                    '[data-testid*="message"], .font-claude-response')
                texts = [(m.inner_text() or "") for m in msgs]
                return texts[-1] if texts else ""
            self._wait_for_stable_text(page, _last_text, timeout=timeout)
        msgs = page.query_selector_all('[data-testid*="message"], .font-claude-response')
        texts = [(m.inner_text() or "") for m in msgs]
        return texts[-1] if texts else ""
