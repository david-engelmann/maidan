"""Selector registry: every selector in one place, named, with fallbacks.

When a lookup fails, the error names the selector key, the URL, and dumps
the nearest stable landmark found — that's the SELECTOR_STALE contract.
Keys are semantic (what the element IS), values are ordered fallback lists
(most specific first). Rot is expected; repair means editing this file.
"""
from __future__ import annotations

# --- auth markers -------------------------------------------------------
# Two independent logged-in markers are required for a logged-in verdict.
LOGGED_IN_MARKERS = {
    # The chat composer: present only in an authenticated session.
    "composer": [
        'div[contenteditable="true"]',
        'textarea[placeholder*="message" i]',
        'textarea',
    ],
    # Secondary: user/account menu or new-chat affordance.
    "account_or_new_chat": [
        'button[aria-label*="profile" i]',
        'button[aria-label*="account" i]',
        '[data-testid="user-menu"]',
        'a[href="/new"]',
        'button:has-text("New chat")',
    ],
}

LOGGED_OUT_MARKERS = {
    "login_button": [
        'a[href*="login"]',
        'button:has-text("Log in")',
        'button:has-text("Sign in")',
    ],
    "login_url": [],  # URL-based: "/login" in page.url
}

BOT_MARKERS = {
    "captcha": [
        'iframe[src*="captcha"]',
        'iframe[src*="challenge"]',
        '[data-testid*="captcha"]',
        'text=/verify you are human/i',
        'text=/unusual traffic/i',
    ],
}

# --- setup flow ----------------------------------------------------------
CUSTOMIZE_BUTTON = [
    'button:has-text("Customize")',
    '[aria-label*="Customize" i]',
]
CONNECTORS_ITEM = [
    'text=Connectors',
    '[href*="connector"]',
]
ADD_CONNECTOR_BUTTON = [
    'button:has-text("Add custom connector")',
    'button[aria-label*="Add connector" i]',
]
CONNECTOR_NAME_INPUT = [
    'input[name="name"]',
    'input[placeholder*="name" i]',
]
CONNECTOR_URL_INPUT = [
    'input[name="url"]',
    'input[placeholder*="URL" i]',
    'input[type="url"]',
]
ADD_BUTTON = [
    'button:has-text("Add")',
]
CONNECT_BUTTON = [
    'button:has-text("Connect")',
]
DISCONNECT_BUTTON = [
    'button:has-text("Disconnect")',
]

# --- test flow ------------------------------------------------------------
TOKEN_FIELD = [
    '#token',
    'input[id="token"]',
]
