"""Magic-link auth: self-healing browser logins via sign-in links.

When a browser provider's saved session expires, the harness can re-login
without a human: trigger the provider's "email me a sign-in" flow, poll
Gmail for the sign-in email, extract the single sign-in LINK, and navigate
to it in the same browser context. Then the existing two-marker auth
verdict confirms the session.

The mechanism is link-only by design (David, 2026-10-07): the login emails
contain one sign-in link; there is no code-entry UI to drive. This is
simpler and more robust than code entry.

Provider shapes (verified vs documented):
- claude: VERIFIED against a real sign-in email, 2026-10-07. "Continue with
  email" sends "Your secure link to Claude.ai is here | <timestamp>" FROM
  zyla@beatgig.com (Anthropic's mail arrives via the Zyla relay, not
  directly from mail.anthropic.com). The "Sign in" button links to
  claude.ai directly. Clicking the link in the SAME browser context that
  requested it auto-logs-in.
- chatgpt: UNVERIFIED. Standard login is email+password; the poller
  supports a sign-in-link email if the account offers one, and the login
  flow fails fast with guidance if a password screen appears instead.
  Confirm the sender/subject against the first real email before enabling.

Mailbox: the test accounts live under zyla@beatgig.com, which David links
as a separate Gmail account. The poller takes an optional `account`
(account_id for `hatch_gws_cli --account`); None means the default
account. Build does not block on the mailbox being linked: link
extraction is unit-tested with realistic fixtures.

Security (hard rules, enforced by tests/test_harness.py):
- The sign-in link is a bearer credential: memory-only, never printed,
  never logged, never persisted. Not in reports, not in stderr, not in
  poller debug output, not in Gmail-saved markdown copies (the poller
  uses the raw Gmail API, never the `+read` shortcut which auto-saves).
- Gmail is polled with tight queries (to: + from: + recency); only the
  newest matching message body is fetched, and only its link extracted.
- The poller creates, labels, forwards, or deletes nothing.
"""
from __future__ import annotations

import base64
import json
import re
import subprocess
import time
from typing import NamedTuple
from urllib.parse import urlparse


class MagicLinkError(Exception):
    """Base for magic-link flow failures (config, extraction, flow shape)."""


class MagicLinkConfigError(MagicLinkError):
    """The magic_link config block is unusable (not enabled, bad email, no
    flow). Surfaces as CONFIG_ERROR (exit 11), not TEST_FAILED: it is a
    setup problem, not a test assertion."""


class CodeTimeout(MagicLinkError):
    """No sign-in email arrived within the poll window.

    Named for the failure-signal registry (CODE_TIMEOUT); the mechanism
    is link-based, but the signal name stays stable.
    """


class SigninLink(NamedTuple):
    url: str  # the sign-in link: memory-only, never log
    message_id: str  # Gmail id, safe to log


# Provider table: sender domain for the Gmail from: search, subject
# matcher, and link-extraction hints. "verified" marks shapes confirmed
# against real email (not guessed).
#
# Claude, verified 2026-10-07 against a real sign-in email: Anthropic's
# mail arrives via the Zyla relay, so the From is zyla@beatgig.com (NOT
# mail.anthropic.com). Match on the relay domain + subject prefix; the
# sign-in button links to claude.ai directly.
PROVIDER_TABLE = {
    "claude": {
        "verified": True,
        "sender_domain": "beatgig.com",
        "subject_prefix": "Your secure link to Claude.ai is here",
        "link_domains": ("claude.ai",),
        "link_text_re": r"sign\s*in",
    },
    "chatgpt": {
        "verified": False,  # UNVERIFIED: confirm against the first real email
        "sender_domain": "openai.com",
        "subject_keywords": ("sign in", "log in", "verify"),
        "link_domains": (),
        "link_text_re": r"sign\s*in|log\s*in|verify|continue",
    },
}

GWS = "hatch_gws_cli"
# Clock skew allowance: accept mail stamped slightly before we triggered.
SKEW_S = 30


def _gws_json(argv: list[str], account: str | None = None) -> dict:
    """Run hatch_gws_cli and parse its JSON stdout. Raises MagicLinkError."""
    cmd = [GWS] + argv
    if account:
        cmd += ["--account", account]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True,
                           timeout=30, stdin=subprocess.DEVNULL)
    except FileNotFoundError:
        raise MagicLinkError(f"{GWS} not found on PATH; cannot poll Gmail")
    except subprocess.TimeoutExpired:
        raise MagicLinkError(
            "hatch_gws_cli timed out after 30s; check the Gmail connector "
            "status and retry")
    if r.returncode != 0:
        raise MagicLinkError(
            f"hatch_gws_cli failed (exit {r.returncode}): "
            f"{(r.stderr or '').strip()[:200]}")
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        # The CLI sometimes exits 0 with a human-readable error on stdout
        # (e.g. "the account may not be linked"). Surface its first line
        # instead of swallowing it.
        first = (r.stdout or "").strip().splitlines()
        hint = f": {first[0][:200]}" if first else ""
        raise MagicLinkError(f"hatch_gws_cli returned non-JSON output{hint}")


def _list_ids(query: str, max_results: int = 5,
              account: str | None = None) -> list[str]:
    data = _gws_json(["gmail", "users", "messages", "list",
                      "--params",
                      json.dumps({"userId": "me", "q": query,
                                  "maxResults": max_results})],
                     account=account)
    return [m["id"] for m in data.get("messages", []) or []]


def _metadata(mid: str, account: str | None = None) -> dict:
    data = _gws_json(["gmail", "users", "messages", "get",
                      "--params",
                      json.dumps({"userId": "me", "id": mid,
                                  "format": "metadata",
                                  "metadataHeaders": ["From", "Subject", "Date"]})],
                     account=account)
    headers = {h["name"].lower(): h["value"]
               for h in data.get("payload", {}).get("headers", [])}
    return {
        "id": mid,
        "from": headers.get("from", ""),
        "subject": headers.get("subject", ""),
        "internal_ms": int(data.get("internalDate", "0") or 0),
    }


def _body_html(mid: str, account: str | None = None) -> str:
    """Fetch the full message and return its HTML (or plain text) in memory.

    Never touches disk: raw API only, no `+read` (which auto-saves markdown).
    """
    data = _gws_json(["gmail", "users", "messages", "get",
                      "--params",
                      json.dumps({"userId": "me", "id": mid, "format": "full"})],
                     account=account)

    def walk(part: dict):
        yield part
        for sub in part.get("parts", []) or []:
            yield from walk(sub)

    html = ""
    text = ""
    for part in walk(data.get("payload", {})):
        mime = part.get("mimeType", "")
        b64 = (part.get("body", {}) or {}).get("data", "")
        if not b64:
            continue
        try:
            decoded = base64.urlsafe_b64decode(b64 + "=" * (-len(b64) % 4))
            decoded = decoded.decode("utf-8", errors="replace")
        except Exception:
            continue
        if mime == "text/html" and not html:
            html = decoded
        elif mime == "text/plain" and not text:
            text = decoded
    return html or text


def _host_ok(href: str, domains: tuple[str, ...]) -> bool:
    """Strict hostname allowlist check.

    Substring matching is NOT enough: ``claude.ai.evil.com`` contains
    ``claude.ai`` but is not Claude. Accept the exact host or a real
    subdomain (``x.claude.ai``), nothing else.
    """
    if not domains or not href.startswith("https://"):
        return False
    try:
        host = (urlparse(href).hostname or "").lower()
    except Exception:
        return False
    return any(host == d or host.endswith("." + d) for d in domains)


def _extract_link(html: str, domains: tuple[str, ...],
                  text_re: str) -> str | None:
    """Pick the sign-in link: anchor text matches, href on the allowlist.

    Every candidate URL must pass `_host_ok` — there is deliberately no
    "any https domain" fallback. A sign-in email whose link points outside
    the provider's allowlist is shape drift (or a spoof); the poller keeps
    polling for a real one instead of navigating somewhere untrusted.

    Never logs the href (it is a Bearer <redacted>).
    """
    anchors = re.findall(
        r'<a\s[^>]*href="([^"]+)"[^>]*>(.*?)</a>',
        html, flags=re.IGNORECASE | re.DOTALL)

    def matching_text(inner: str) -> bool:
        text = re.sub(r"<[^>]+>", "", inner)
        return bool(re.search(text_re, text, re.IGNORECASE))

    # Pass 1: matching anchor text + allowlisted domain.
    for href, inner in anchors:
        if matching_text(inner) and _host_ok(href, domains):
            return href
    # Pass 2: markdown-style [text](url), for text/plain fallbacks.
    for m in re.finditer(r"\[([^\]]*)\]\((https://[^)]+)\)", html):
        if re.search(text_re, m.group(1), re.IGNORECASE) \
                and _host_ok(m.group(2), domains):
            return m.group(2)
    # Pass 3: bare https URL on an allowlisted domain (plain-text emails).
    # Strip trailing punctuation the regex sweeps up ("...verify.").
    for m in re.finditer(r"https://[^\s\"'<>]+", html):
        url = m.group(0).rstrip(".,;:!?)]}\"'")
        if _host_ok(url, domains):
            return url
    return None


def _subject_matches(provider: str, subject: str) -> bool:
    spec = PROVIDER_TABLE[provider]
    if spec.get("subject_prefix"):
        return subject.startswith(spec["subject_prefix"])
    kws = spec.get("subject_keywords", ())
    low = subject.lower()
    return any(k in low for k in kws)


def poll_for_signin(provider: str, email: str, since_epoch: float,
                    timeout: float = 120, poll_interval: float = 5,
                    account: str | None = None,
                    allow_unverified: bool = False,
                    link_domains: tuple[str, ...] | None = None,
                    _fetch=None) -> SigninLink:
    """Wait for the provider's sign-in email and extract its link.

    Returns SigninLink(url, message_id). The URL is memory-only: this
    function never prints or logs it. Raises CodeTimeout after `timeout`
    seconds. Raises MagicLinkError on Gmail failures.

    `account` is the Gmail account_id for `hatch_gws_cli --account`;
    None means the default account.

    Providers whose email shape is UNVERIFIED (see PROVIDER_TABLE) are
    refused unless `allow_unverified` is True AND `link_domains` gives an
    explicit hostname allowlist. Both default to fail-closed: the harness
    never navigates to a link extracted from a guessed email shape.

    `_fetch` is an injection hook for tests: a callable
    (query) -> list of metadata dicts; entries may carry a "body" key,
    used directly instead of fetching via Gmail. When given, no Gmail
    calls happen.
    """
    if provider not in PROVIDER_TABLE:
        raise MagicLinkError(f"no magic-link table entry for {provider!r}")
    spec = PROVIDER_TABLE[provider]
    if not spec.get("verified") and not allow_unverified:
        raise MagicLinkConfigError(
            f"{provider}: sign-in email shape is UNVERIFIED — confirm the "
            f"real sender/subject/link domains against an actual email "
            f"first, then set "
            f"providers.{provider}.magic_link.allow_unverified=true "
            f"(and .link_domains to the confirmed domains). Refusing to "
            f"act on a guessed email shape.")
    domains = tuple(link_domains) if link_domains else spec["link_domains"]
    if not domains:
        raise MagicLinkConfigError(
            f"{provider}: no link-domain allowlist configured; refusing to "
            f"extract links without one. Set "
            f"providers.{provider}.magic_link.link_domains.")
    query = (f"to:{email} from:{spec['sender_domain']} newer_than:1h")
    seen: set[str] = set()
    deadline = time.time() + timeout

    def fetch_candidates() -> list[dict]:
        if _fetch is not None:
            return [m for m in _fetch(query)
                    if m["internal_ms"] / 1000 >= since_epoch - SKEW_S
                    and _subject_matches(provider, m["subject"])]
        out = []
        for mid in _list_ids(query, account=account):
            if mid in seen:
                continue
            seen.add(mid)
            meta = _metadata(mid, account=account)
            if meta["internal_ms"] / 1000 < since_epoch - SKEW_S:
                continue
            if not _subject_matches(provider, meta["subject"]):
                continue
            out.append(meta)
        return out

    def fetch_body(mid: str, cached: dict) -> str:
        if cached.get("body") is not None:
            return cached["body"]
        return _body_html(mid, account=account)

    while True:
        candidates = fetch_candidates()
        if candidates:
            candidates.sort(key=lambda m: m["internal_ms"], reverse=True)
            newest = candidates[0]
            url = _extract_link(fetch_body(newest["id"], newest),
                                domains, spec["link_text_re"])
            if url:
                return SigninLink(url, newest["id"])
            # Newest matching mail had no extractable link (wrong mail or
            # shape drift): keep polling for a fresher one.
        if time.time() >= deadline:
            acct = f" on Gmail account {account}" if account else ""
            raise CodeTimeout(
                f"no {provider} sign-in email for {email}{acct} within "
                f"{int(timeout)}s (query matched nothing extractable; if the "
                f"account is wrong, set providers.{provider}.magic_link.account)")
        time.sleep(poll_interval)
