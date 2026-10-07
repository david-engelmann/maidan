"""ChatGPT provider: dev-mode MCP plugin.

Research (§1.1): developer mode is web-only and excludes the Free tier.
Install: Settings → Security and login → enable Developer mode, then the
Plugins page → create a developer-mode app for the MCP server URL (draft).
First call: new chat → composer `+` menu → Developer mode → select the
dev app → run an action. OAuth fires lazily on first TOOL use, not at
connect; no-auth covers install/discovery/reads.

Advanced features:
- MCP Apps / widgets: widget_present() probes for the rendered iframe
  (best-effort; the server must emit a widget payload).
- Conversational approval: NO elicitation/create on ChatGPT either.
- Multi-tool agentic sequence.
"""
from __future__ import annotations

from .. import config as config_mod
from ..selectors import find, any_present
from .base import Provider
from . import selectors_chatgpt as S


class ChatGPTProvider(Provider):
    name = "chatgpt"
    app_url = "https://chatgpt.com"

    # -- auth -----------------------------------------------------------
    def auth_markers(self):
        return (S.LOGGED_IN_MARKERS, S.LOGGED_OUT_MARKERS, S.BOT_MARKERS)

    def logged_out_url(self, url: str) -> bool:
        url = url or ""
        return "auth.openai.com" in url or "/login" in url

    def is_logged_in(self, page) -> bool:  # legacy shim for auth-login UX
        from ..auth import check_auth_markers
        return check_auth_markers(page, self)["verdict"] == "logged-in"

    # -- setup ----------------------------------------------------------
    def _open_settings(self, page):
        find(page, "profile_button", S.PROFILE_BUTTON).click()
        find(page, "settings_item", S.SETTINGS_ITEM).click()

    def is_setup(self, page) -> bool:
        return self.setup_probe(page)["ok"]

    def setup_probe(self, page) -> dict:
        """Cheap probe: dev-mode row present AND draft plugin listed.

        URL verification: the Plugins page row rarely exposes the MCP URL
        without opening the editor, so a name match counts as ok with
        detail "URL unverified". Inconclusive -> ok=False (SETUP_NEEDED,
        not a 5-minute test timeout).
        """
        name = self.pcfg["plugin_name"]
        try:
            self._open_settings(page)
            find(page, "security_tab", S.SECURITY_TAB, timeout=8000).click()
            if not any_present(page, S.DEVELOPER_MODE_ROW, timeout=5000):
                return {"ok": False, "detail": "Developer mode row not found"}
            page.goto(f"{self.app_url}/plugins", wait_until="domcontentloaded")
            try:
                page.wait_for_selector(f'text="{name}"', timeout=8000)
            except Exception:
                return {"ok": False,
                        "detail": f"draft plugin {name!r} not listed"}
            return {"ok": True,
                    "detail": f"draft plugin {name!r} listed (URL unverified)"}
        except SystemExit:
            raise
        except Exception as e:
            return {"ok": False, "detail": f"probe inconclusive: {e}"}

    def install(self, page) -> None:
        mcp = config_mod.mcp_url(self.cfg)
        name = self.pcfg["plugin_name"]
        self._open_settings(page)
        find(page, "security_tab", S.SECURITY_TAB).click()
        devmode = find(page, "developer_mode_row", S.DEVELOPER_MODE_ROW)
        switch = devmode.evaluate_handle(
            "el => el.closest('[role=\"switch\"], button') || el")
        if switch:
            try:
                if switch.get_attribute("aria-checked") == "false":
                    switch.click()
            except Exception:
                pass
        page.goto(f"{self.app_url}/plugins", wait_until="domcontentloaded")
        find(page, "create_button", S.CREATE_BUTTON).click()
        find(page, "plugin_name_input", S.PLUGIN_NAME_INPUT).fill(name)
        find(page, "plugin_url_input", S.PLUGIN_URL_INPUT).fill(mcp)
        find(page, "create_button", S.CREATE_BUTTON).click()
        find(page, "plugin_row", [f'text="{name}"'], timeout=15000)

    # -- test -----------------------------------------------------------
    def new_chat(self, page) -> None:
        page.goto(f"{self.app_url}/", wait_until="domcontentloaded")
        find(page, "composer", S.LOGGED_IN_MARKERS["composer"], timeout=15000)

    def _enable_dev_app(self, page) -> None:
        name = self.pcfg["plugin_name"]
        find(page, "attach_button", S.ATTACH_BUTTON).click()
        find(page, "developer_mode_menu_item", S.DEVELOPER_MODE_MENU_ITEM).click()
        find(page, "dev_app_item", [f'text="{name}"']).click()

    def send_prompt(self, page, prompt: str) -> None:
        self._enable_dev_app(page)
        box = find(page, "composer", S.LOGGED_IN_MARKERS["composer"])
        box.click()
        box.fill(prompt)
        box.press("Enter")

    def wait_for_response(self, page, timeout: float = 120.0) -> str:
        try:
            page.wait_for_selector('[data-testid="stop-button"]', timeout=15000)
            page.wait_for_selector('[data-testid="stop-button"]',
                                   state="detached",
                                   timeout=int(timeout * 1000))
        except Exception:
            # Stop button never appeared: poll until the response text
            # stabilizes instead of a blind sleep.
            def _last_text():
                msgs = page.query_selector_all(
                    '[data-testid="conversation-turn-2"] [data-message-author-role="assistant"]')
                texts = [(m.inner_text() or "") for m in msgs]
                return texts[-1] if texts else ""
            self._wait_for_stable_text(page, _last_text, timeout=timeout)
        msgs = page.query_selector_all(
            '[data-testid="conversation-turn-2"] [data-message-author-role="assistant"]')
        texts = [(m.inner_text() or "") for m in msgs]
        return texts[-1] if texts else ""

    def widget_present(self, page) -> bool:
        """Best-effort MCP Apps check: is a widget iframe rendered?"""
        try:
            frames = page.query_selector_all(
                'iframe[src*="openai"], iframe[title*="app" i]')
            return len(frames) > 0
        except Exception:
            return False
