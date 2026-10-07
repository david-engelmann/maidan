"""Selector registry for ChatGPT (chatgpt.com). Same contract as Claude's:
named keys, ordered fallbacks, SELECTOR_STALE names key + URL + landmark.
"""
from __future__ import annotations

# --- auth markers -------------------------------------------------------
LOGGED_IN_MARKERS = {
    # The home composer: present only in an authenticated session.
    "composer": [
        "#prompt-textarea",
        'div[contenteditable="true"]',
        'textarea[placeholder*="Message" i]',
    ],
    "account_or_sidebar": [
        '[data-testid="profile-button"]',
        'nav[aria-label*="Chat history" i]',
        '[data-testid*="new-chat"]',
        'button[aria-label*="New chat" i]',
    ],
}

LOGGED_OUT_MARKERS = {
    "login_button": [
        'button:has-text("Log in")',
        'button:has-text("Sign up")',
        'a[href*="auth.openai.com"]',
    ],
    "login_url": [],  # URL-based: "auth.openai.com" or "/login" in page.url
}

BOT_MARKERS = {
    "captcha": [
        'iframe[src*="captcha"]',
        'iframe[src*="challenge"]',
        'text=/verify you are human/i',
        'text=/unusual traffic/i',
    ],
}

# --- setup flow ----------------------------------------------------------
PROFILE_BUTTON = [
    '[data-testid="profile-button"]',
    'button[aria-label*="profile" i]',
]
SETTINGS_ITEM = [
    'text=Settings',
]
SECURITY_TAB = [
    'text=Security',
    'text=/security and login/i',
]
DEVELOPER_MODE_ROW = [
    'text=Developer mode',
]
PLUGIN_NAME_INPUT = [
    'input[name="name"]',
    'input[placeholder*="name" i]',
]
PLUGIN_URL_INPUT = [
    'input[name*="url" i]',
    'input[placeholder*="URL" i]',
]
CREATE_BUTTON = [
    'button:has-text("Create")',
    'button:has-text("New")',
    'button:has-text("Save")',
]

# --- test flow ------------------------------------------------------------
ATTACH_BUTTON = [
    '[aria-label*="Attach" i]',
    'button:has-text("+")',
    '[data-testid*="composer-plus"]',
]
DEVELOPER_MODE_MENU_ITEM = [
    'text=Developer mode',
]
STOP_BUTTON = [
    '[data-testid="stop-button"]',
]
