"""Shared selector lookup with the SELECTOR_STALE contract.

_find(page, key, fallbacks, timeout): tries each fallback selector in
order; on total failure prints
  SELECTOR_STALE: <key> tried [<s1>, <s2>] @ <url> :: landmark: <...>
and exits 5. The landmark is the nearest stable thing found: the page
title plus the first few button/link texts, so a human repairing the
selector knows what the page actually looked like.
"""
from __future__ import annotations

import sys

from . import SELECTOR_STALE, EXIT_SELECTOR


def nearest_landmark(page, max_items: int = 8) -> str:
    """Best-effort description of what the page actually contains."""
    try:
        title = page.title() or ""
    except Exception:
        title = ""
    bits = []
    if title:
        bits.append(f"title={title[:60]!r}")
    try:
        texts = page.evaluate(
            """() => Array.from(document.querySelectorAll(
                'button, a, h1, h2, input[placeholder]'))
                .slice(0, 40)
                .map(el => (el.innerText || el.placeholder || el.getAttribute('aria-label') || '').trim())
                .filter(t => t.length > 0 && t.length < 60)"""
        )
        seen = []
        for t in texts:
            if t not in seen:
                seen.append(t)
            if len(seen) >= max_items:
                break
        if seen:
            bits.append("ui=" + " | ".join(seen))
    except Exception as e:
        bits.append(f"ui-unreadable({e})")
    return "; ".join(bits) if bits else "(empty/unreadable page)"


def find(page, key: str, fallbacks: list[str], timeout: int = 8000):
    """Return the first matching element or die with SELECTOR_STALE."""
    url = ""
    try:
        url = page.url
    except Exception:
        pass
    for sel in fallbacks:
        try:
            el = page.wait_for_selector(sel, timeout=timeout, state="attached")
            if el is not None:
                return el
        except Exception:
            continue
    landmark = nearest_landmark(page)
    print(f"{SELECTOR_STALE}: {key} tried {fallbacks} @ {url} :: landmark: {landmark}",
          file=sys.stderr)
    sys.exit(EXIT_SELECTOR)


def any_present(page, fallbacks: list[str], timeout: int = 4000) -> bool:
    """True if any fallback matches within the timeout. Never dies."""
    for sel in fallbacks:
        try:
            if page.wait_for_selector(sel, timeout=timeout, state="attached"):
                return True
        except Exception:
            continue
    return False
