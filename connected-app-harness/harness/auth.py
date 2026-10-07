"""Auth: one manual login, then never block on auth again.

The highest-risk seam is auth-state detection: a naive "look for avatar"
check flakes on slow loads and false-positives on cached shells. So
check-auth runs a small state machine with explicit waits:

  1. Navigate to the app (short timeout; nav failure -> verdict below).
  2. Bot-detection markers first: captcha/challenge interstitial ->
     AUTH_EXPIRED (detail: bot-detection). Never interact with it.
  3. Logged-out markers: login URL or login button -> AUTH_EXPIRED.
  4. Logged-in markers: at least TWO independent markers required
     (e.g. composer + account menu). One marker alone is not enough —
     cached shells can show one.
  5. Definite timeout (default 25s): anything else -> AUTH_EXPIRED.

The verdict records which markers were seen/missing, and the report
carries them. check_auth_or_die exits 3 on anything but logged-in, and
never waits at a login screen, never retries, never prompts.
"""
from __future__ import annotations

import sys
import time

from . import AUTH_EXPIRED, EXIT_AUTH
from .browser import persistent_context
from . import config as config_mod
from .magiclink import (
    MagicLinkError, MagicLinkConfigError, CodeTimeout, poll_for_signin,
)
from .selectors import any_present, find

AUTH_TIMEOUT_S = 25

# Login-page selectors (the provider login screens, not the app UI — kept
# here, not in the app selector registries). Fallback order: most specific
# first. Misses die with SELECTOR_STALE via find().
_LOGIN_SELECTORS = {
    "claude": {
        # https://claude.ai/login — "Continue with email" sends the
        # "Your secure link to Claude.ai is here" email (verified
        # 2026-10-07; arrives via the zyla@beatgig.com relay).
        "email_input": [
            'input[type="email"]',
            'input[name="email"]',
        ],
        "continue_button": [
            'button:has-text("Continue with email")',
            'button[type="submit"]',
        ],
        # Shown after submit while the email is on its way.
        "sent_indicator": [
            'text=/check your email/i',
            'text=/secure link/i',
        ],
    },
    "chatgpt": {
        # https://chatgpt.com/auth/login — email-first; UNVERIFIED whether
        # the account offers a sign-in-link email (standard is
        # email+password). A password field fails fast with guidance.
        "email_input": [
            'input[type="email"]',
            'input[name="username"]',
            '#username',
        ],
        "continue_button": [
            'button[type="submit"]',
            'button:has-text("Continue")',
        ],
        "password_input": [
            'input[type="password"]',
        ],
        # Shown after submit while the email is on its way (best-effort).
        "sent_indicator": [
            'text=/check your email/i',
            'text=/click the link in the email/i',
        ],
    },
}

_LOGIN_URLS = {
    "claude": "https://claude.ai/login",
    # chatgpt.com/auth/login is hard-blocked by Cloudflare from some
    # networks; auth.openai.com/log-in loads clean (verified 2026-10-07).
    "chatgpt": "https://auth.openai.com/log-in",
}


def check_auth_markers(page, provider) -> dict:
    """Run the marker state machine. Returns a verdict dict.

    {
      "verdict": "logged-in" | "logged-out" | "bot-detected" | "unknown",
      "seen": {marker_key: selector_that_matched},
      "missing": [marker_keys...],
      "elapsed_s": float,
    }
    """
    t0 = time.time()
    seen: dict[str, str] = {}
    logged_in, logged_out, bot = provider.auth_markers()

    def scan(markers: dict[str, list[str]], timeout_ms: int) -> dict[str, str]:
        found: dict[str, str] = {}
        for key, fallbacks in markers.items():
            if not fallbacks:  # URL-based markers handled by caller
                continue
            for sel in fallbacks:
                try:
                    if page.wait_for_selector(sel, timeout=timeout_ms,
                                              state="attached"):
                        found[key] = sel
                        break
                except Exception:
                    continue
        return found

    # Pass 1 (fast): bot detection and logged-out markers get short waits —
    # if they match, we know immediately without burning the budget.
    bot_found = scan(bot, 2500)
    if bot_found:
        return {"verdict": "bot-detected", "seen": bot_found,
                "missing": [], "elapsed_s": round(time.time() - t0, 1)}
    # URL-based logged-out markers (cheap, no wait).
    url = ""
    try:
        url = page.url
    except Exception:
        pass
    if provider.logged_out_url(url):
        return {"verdict": "logged-out",
                "seen": {"login_url": url},
                "missing": [], "elapsed_s": round(time.time() - t0, 1)}
    out_found = scan(logged_out, 2500)
    if out_found:
        return {"verdict": "logged-out", "seen": out_found,
                "missing": [], "elapsed_s": round(time.time() - t0, 1)}

    # Pass 2: logged-in markers get the remaining budget. TWO independent
    # markers are required — one can be a cached shell.
    budget_ms = max(1000, int((AUTH_TIMEOUT_S - (time.time() - t0)) * 1000))
    per_marker = max(1000, budget_ms // max(1, len(logged_in)))
    in_found = scan(logged_in, per_marker)
    missing = [k for k in logged_in if k not in in_found]
    verdict = "logged-in" if len(in_found) >= 2 else "unknown"
    return {"verdict": verdict, "seen": in_found, "missing": missing,
            "elapsed_s": round(time.time() - t0, 1)}


def auth_login(provider: str, cfg: dict) -> None:
    """Open a headed browser for David's one-time manual login."""
    from .providers import get_provider

    prov = get_provider(provider, cfg)
    print(f"=== One-time login for {provider} ===")
    print(f"1. A browser window will open at {prov.app_url}.")
    print("2. Log in manually (complete any 2FA / SSO yourself).")
    print("3. Wait until you see the normal chat UI, then press Enter here.")
    print("4. The profile is saved; you should never need this again.")
    print()
    input("Press Enter to open the browser...")
    with persistent_context(provider, headless=False) as ctx:
        page = ctx.new_page()
        page.goto(prov.app_url, wait_until="domcontentloaded", timeout=30000)
        input("Log in in the browser window, then press Enter here when done...")
        verdict = check_auth_markers(page, prov)
        if verdict["verdict"] == "logged-in":
            print(f"OK: {provider} session saved to {config_mod.profile_dir(provider)}")
            print(f"    markers seen: {verdict['seen']}")
        else:
            print(
                f"WARNING: {provider} verdict={verdict['verdict']} "
                f"(seen={verdict['seen']}, missing={verdict['missing']}). "
                "Profile saved anyway; re-run auth-login if check-auth fails.",
                file=sys.stderr,
            )


def check_auth_or_die(provider: str, cfg: dict) -> dict:
    """Fail fast with AUTH_EXPIRED unless the saved session is alive.

    Returns the verdict dict on success (callers log it in the report).
    """
    from .providers import get_provider

    prov = get_provider(provider, cfg)
    # Fail before launching a browser when there's no profile at all.
    import os
    if not os.path.isdir(config_mod.profile_dir(provider)):
        print(AUTH_EXPIRED, file=sys.stderr)
        print(f"{provider}: no saved profile; "
              f"run `harness auth-login {provider}` once, then retry.",
              file=sys.stderr)
        sys.exit(EXIT_AUTH)
    with persistent_context(provider, headless=True) as ctx:
        page = ctx.new_page()
        try:
            page.goto(prov.app_url, wait_until="domcontentloaded", timeout=20000)
        except Exception:
            pass  # nav hiccups still get a marker scan below
        verdict = check_auth_markers(page, prov)
        if verdict["verdict"] == "logged-in":
            return verdict
    detail = (f"verdict={verdict['verdict']} seen={verdict['seen']} "
              f"missing={verdict['missing']} in {verdict['elapsed_s']}s")
    print(AUTH_EXPIRED, file=sys.stderr)
    print(f"{provider}: {detail}. Run `harness auth-login {provider}` once, "
          f"then retry.", file=sys.stderr)
    sys.exit(EXIT_AUTH)


def _magic_link_config(provider: str, cfg: dict,
                       require_enabled: bool = True) -> dict:
    """Return the magic_link config block; raise if unusable.

    `require_enabled=False` is the explicit one-shot path (`--via-email`):
    the human is right here asking for it, so the config default doesn't
    gate it. The nightly self-heal always requires enabled.
    """
    ml = ((cfg.get("providers") or {}).get(provider) or {}).get("magic_link") or {}
    if require_enabled and not ml.get("enabled"):
        raise MagicLinkConfigError(
            f"{provider}: magic_link not enabled in config "
            f"(providers.{provider}.magic_link.enabled). Create the "
            f"zyla@beatgig.com provider account first (one-time human "
            f"step), link that Gmail account, then enable it — see README.md.")
    email = (ml.get("email") or "").strip()
    if "@" not in email:
        raise MagicLinkConfigError(
            f"{provider}: magic_link.email is missing or invalid in config.")
    if provider not in _LOGIN_URLS:
        raise MagicLinkConfigError(
            f"{provider}: no magic-link login flow implemented")
    # Gmail account_id for --account; None = default account.
    account = (ml.get("account") or "").strip() or None
    # Unverified-shape override: explicit opt-in only, plus the operator-
    # confirmed link-domain allowlist. Both default to fail-closed.
    allow_unverified = bool(ml.get("allow_unverified", False))
    link_domains = tuple(ml.get("link_domains") or ())
    return {"email": email, "account": account,
            "allow_unverified": allow_unverified,
            "link_domains": link_domains}


def magic_link_login(provider: str, cfg: dict,
                       bypass_enabled_gate: bool = False) -> dict:
    """Headless email-based re-login via a single sign-in link.

    Zero human in the loop. Requires
    providers.<provider>.magic_link.enabled=true, a valid .email, and the
    Gmail account linked (magic_link.account, or the default account).
    Uses the provider's PERSISTENT profile so the new session is saved
    for later check-auth/test runs.

    Flow: submit the email on the provider login page -> poll Gmail for
    the sign-in email -> navigate to its link in the SAME browser context
    -> two-marker auth verdict.

    Returns the logged-in verdict dict. Raises CodeTimeout when the
    sign-in email never arrives (caller maps to CODE_TIMEOUT), raises
    MagicLinkError on config/flow failures, and exits AUTH_EXPIRED when
    the flow completes but the marker check is not logged-in (mirroring
    check_auth_or_die).
    """
    from .providers import get_provider

    ml = _magic_link_config(provider, cfg,
                              require_enabled=not bypass_enabled_gate)
    email = ml["email"]
    account = ml["account"]
    allow_unverified = ml["allow_unverified"]
    link_domains = ml["link_domains"]
    prov = get_provider(provider, cfg)
    S = _LOGIN_SELECTORS[provider]

    with persistent_context(provider, headless=True) as ctx:
        page = ctx.new_page()
        try:
            page.goto(_LOGIN_URLS[provider], wait_until="domcontentloaded",
                      timeout=30000)
        except Exception as e:
            raise MagicLinkError(
                f"{provider}: login page unreachable: {e}") from e

        find(page, "login_email_input", S["email_input"]).fill(email)
        since_epoch = time.time()  # the sign-in mail must be newer than this
        find(page, "login_continue_button", S["continue_button"]).click()

        if provider == "claude":
            _consume_signin_link(page, S, "claude", email, since_epoch,
                                 account, allow_unverified, link_domains)
        elif provider == "chatgpt":
            _chatgpt_consume_link(page, S, email, since_epoch, account,
                                  allow_unverified, link_domains)
        else:  # pragma: no cover - guarded by _magic_link_config
            raise MagicLinkError(f"{provider}: no login flow implemented")

        verdict = check_auth_markers(page, prov)
        if verdict["verdict"] == "logged-in":
            return verdict
    detail = (f"verdict={verdict['verdict']} seen={verdict['seen']} "
              f"missing={verdict['missing']}")
    print(AUTH_EXPIRED, file=sys.stderr)
    print(f"{provider}: magic-link flow finished but session not logged in "
          f"({detail}).", file=sys.stderr)
    sys.exit(EXIT_AUTH)


def _consume_signin_link(page, S: dict, provider: str, email: str,
                         since_epoch: float, account: str | None,
                         allow_unverified: bool = False,
                         link_domains: tuple = ()) -> None:
    """Poll for the sign-in LINK email, open it in this same context.

    The link must be opened in the SAME browser context that requested it
    (per Anthropic's docs, same-device clicks auto-login). The link is a
    bearer credential: memory-only, never logged.
    """
    # Best-effort: wait for the "check your email" state, then poll.
    any_present(page, S["sent_indicator"], timeout=10000)
    secret = poll_for_signin(provider, email, since_epoch, account=account,
                             allow_unverified=allow_unverified,
                             link_domains=link_domains or None)
    try:
        page.goto(secret.url, wait_until="domcontentloaded", timeout=30000)
    except Exception as e:
        # Never include secret.url: Playwright embeds the navigated URL in
        # goto errors, and the link is a live credential. The message names
        # the failure and the next step, never the link.
        raise MagicLinkError(
            f"{provider}: opening the sign-in link failed "
            f"({type(e).__name__}); the link may have expired — "
            f"retry `harness auth-login {provider} --via-email`")


def _chatgpt_consume_link(page, S: dict, email: str, since_epoch: float,
                          account: str | None,
                          allow_unverified: bool = False,
                          link_domains: tuple = ()) -> None:
    """ChatGPT: fail fast on password screens, else consume the sign-in link.

    UNVERIFIED: standard ChatGPT login is email+password. If the account
    offers a sign-in-link email, complete it; if a password field appears,
    fail fast with guidance (magic-link unavailable for this account).
    """
    # Password screen -> magic-link cannot work for this account.
    if any_present(page, S["password_input"], timeout=8000):
        raise MagicLinkError(
            "chatgpt: login page asks for a password; this account has no "
            "sign-in-link email path. Use headed "
            "`harness auth-login chatgpt` once instead.")
    _consume_signin_link(page, S, "chatgpt", email, since_epoch, account,
                         allow_unverified=allow_unverified,
                         link_domains=link_domains)
