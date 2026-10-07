"""Provider registry: browser providers + CLI providers."""
from __future__ import annotations

from .base import Provider
from .cli_base import CLIProvider
from .claude import ClaudeProvider
from .chatgpt import ChatGPTProvider
from .grok import GrokProvider
from .gemini import GeminiProvider
from .copilot import CopilotMCPProvider, CopilotPluginProvider
from .claude_code import ClaudeCodeProvider
from .cursor import CursorProvider
from .meta import MetaProvider

BROWSER_PROVIDERS: dict[str, type[Provider]] = {
    "claude": ClaudeProvider,
    "chatgpt": ChatGPTProvider,
}

CLI_PROVIDERS: dict[str, type[CLIProvider]] = {
    "grok": GrokProvider,
    "gemini": GeminiProvider,
    "copilot-mcp": CopilotMCPProvider,
    "copilot-plugin": CopilotPluginProvider,
    "claude-code": ClaudeCodeProvider,
    "cursor": CursorProvider,
    "meta": MetaProvider,
}

PROVIDERS: dict = {**BROWSER_PROVIDERS, **CLI_PROVIDERS}

# Unattended-capable providers, in nightly order. Manual-only providers
# (cursor, meta) are never in the nightly; they refuse with MANUAL_ONLY.
NIGHTLY_ORDER: list[str] = [
    "claude", "chatgpt",
    "grok", "gemini", "claude-code", "copilot-mcp", "copilot-plugin",
]


def get_provider(name: str, cfg: dict):
    try:
        return PROVIDERS[name](cfg)
    except KeyError:
        raise SystemExit(f"unknown provider {name!r} (choose: {', '.join(PROVIDERS)})")


def is_cli_provider(name: str) -> bool:
    return name in CLI_PROVIDERS


def is_browser_provider(name: str) -> bool:
    return name in BROWSER_PROVIDERS
