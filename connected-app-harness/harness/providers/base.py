"""Provider interface. Every provider implements the same lifecycle:

  check-auth  -> auth_markers() + logged_out_url()   (state machine, fast)
  setup       -> is_setup(page) / install(page)      (idempotent; rare)
  test        -> run_suite(page, suite, ...)          (assumes setup; fast fail)

A provider never logs in, never installs during test, never waits at a
login screen. All selectors live in the provider's selectors_* module;
lookups go through harness.selectors.find (SELECTOR_STALE contract).
"""
from __future__ import annotations


class Provider:
    name = "base"
    app_url = ""
    # Integration surface: "web" (browser), "cli", "api", or "manual".
    # Used by client_info telemetry so reports distinguish runs.
    surface = "web"

    def __init__(self, cfg: dict):
        self.cfg = cfg
        self.pcfg = cfg["providers"][self.name]

    # -- auth -----------------------------------------------------------
    def auth_markers(self) -> tuple[dict, dict, dict]:
        """(logged_in, logged_out, bot) marker dicts for the state machine.

        logged_in needs at least TWO independent keys to match.
        """
        raise NotImplementedError

    def logged_out_url(self, url: str) -> bool:
        """URL-based logged-out detection (no waiting)."""
        raise NotImplementedError

    # -- setup (rare) ----------------------------------------------------
    def is_setup(self, page) -> bool:
        """True if the connector/plugin is installed and reachable."""
        raise NotImplementedError

    def setup_probe(self, page) -> dict:
        """Cheap pre-test probe: is the connector installed AND pointed at
        the configured MCP URL? Returns {"ok": bool, "detail": str}.
        Must not start a chat round-trip."""
        raise NotImplementedError

    def install(self, page) -> None:
        """Install the connector/plugin. Assumes logged in."""
        raise NotImplementedError

    # -- test (often) ----------------------------------------------------
    def new_chat(self, page) -> None:
        raise NotImplementedError

    def send_prompt(self, page, prompt: str) -> None:
        raise NotImplementedError

    def wait_for_response(self, page, timeout: float = 120.0) -> str:
        """Wait for the assistant response; return its text."""
        raise NotImplementedError

    # -- shared helpers --------------------------------------------------
    @staticmethod
    def _wait_for_stable_text(page, text_fn, timeout: float = 60.0,
                              stable_for: float = 3.0) -> None:
        """Poll until text_fn() stops changing (or timeout).

        Replaces blind sleeps: when the stop-button never appears, we wait
        on the response actually stabilizing instead of guessing a duration.
        """
        import time
        deadline = time.time() + timeout
        last = None
        stable_since = time.time()
        while time.time() < deadline:
            try:
                cur = text_fn()
            except Exception:
                cur = None
            if cur != last:
                last = cur
                stable_since = time.time()
            elif time.time() - stable_since >= stable_for and cur:
                return
            time.sleep(0.5)
