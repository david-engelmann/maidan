"""Keeping Maidan's bytes in the provider's cache.

The boot prefix with a cache breakpoint, one cache key per shared-prefix
group, and the thread id as a gateway session id. Pure functions; the
TypeScript, Go and Rust SDKs give the same output for the same input
(``docs/Harness Caching.md``).
"""

from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass
from typing import Any, Union

__all__ = [
    "BootPrefix",
    "CacheError",
    "boot_prefix",
    "cache_key",
    "cache_key_fields",
    "cached_prefix",
    "gateway_session",
]


class CacheError(ValueError):
    """A cache or gateway helper given an input it cannot place."""


@dataclass(frozen=True)
class BootPrefix:
    """The channel's boot prefix, byte for byte as served, and its sha256 (hex)."""

    text: str
    sha256: str


def boot_prefix(data: Union[bytes, str]) -> BootPrefix:
    raw = data.encode("utf-8") if isinstance(data, str) else data
    return BootPrefix(text=raw.decode("utf-8"), sha256=hashlib.sha256(raw).hexdigest())


_TTLS = ("5m", "1h")
_SYSTEM_MESSAGE = ("openai-chat", "deepseek", "mistral", "xai", "vllm")


def cached_prefix(provider: str, text: str, ttl: str | None = None) -> Any:
    """Place ``text`` (the boot prefix) as the first, cached part of a request.

    Anthropic and Bedrock get an explicit breakpoint and take ``ttl`` ("5m" or
    "1h"); OpenAI Responses gets an explicit breakpoint (GPT-5.6 and later, 30
    minutes); the rest cache a matching prefix on their own, so the prefix is
    only put first.
    """
    if ttl is not None and ttl not in _TTLS:
        raise CacheError("ttl must be 5m or 1h")
    if ttl is not None and provider not in ("anthropic", "bedrock-converse"):
        raise CacheError(f"{provider} takes no cache ttl")
    long_ttl = {"ttl": "1h"} if ttl == "1h" else {}
    if provider == "anthropic":
        return {"type": "text", "text": text, "cache_control": {"type": "ephemeral", **long_ttl}}
    if provider == "bedrock-converse":
        return [{"text": text}, {"cachePoint": {"type": "default", **long_ttl}}]
    if provider == "openai-responses":
        return {
            "type": "message",
            "role": "developer",
            "content": [
                {"type": "input_text", "text": text, "prompt_cache_breakpoint": {"mode": "explicit"}}
            ],
        }
    if provider == "gemini":
        return {"parts": [{"text": text}]}
    if provider in _SYSTEM_MESSAGE:
        return {"role": "system", "content": text}
    raise CacheError(f"unknown provider {provider}")


def cache_key(workspace_id: str, group: str) -> str:
    """One key per shared-prefix group, never the same in two workspaces.

    The workspace id is hashed in, so the same group name in two workspaces
    gives two keys, and the key reveals neither.
    """
    if not workspace_id or not group:
        raise CacheError("workspace id and group are required")
    digest = hashlib.sha256(f"{workspace_id}\n{group}".encode("utf-8")).hexdigest()
    return f"maidan-{digest[:32]}"


def cache_key_fields(provider: str, key: str) -> dict:
    """Where ``key`` goes for a provider that takes one; ``{}`` for one that does not."""
    if provider in ("openai-responses", "openai-chat", "mistral", "xai-responses"):
        return {"body": {"prompt_cache_key": key}}
    if provider == "xai-chat":
        return {"headers": {"x-grok-conv-id": key}}
    if provider == "deepseek":
        return {"body": {"user_id": key}}
    if provider == "deepseek-anthropic":
        return {"body": {"metadata": {"user_id": key}}}
    if provider == "vllm":
        return {"body": {"cache_salt": key}}
    if provider in ("anthropic", "bedrock-converse", "gemini"):
        return {}
    raise CacheError(f"unknown provider {provider}")


_UUID_V7 = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$", re.I)


def gateway_session(gateway: str, thread_id: str, path: str = "/", name: str = "maidan") -> dict:
    """The thread id as the gateway's session id, so gateway spend joins the thread.

    Helicone also takes ``path`` and ``name``.
    """
    if not thread_id:
        raise CacheError("thread id is required")
    if gateway == "openrouter":
        return {"body": {"session_id": thread_id}}
    if gateway == "helicone":
        return {
            "headers": {
                "Helicone-Session-Id": thread_id,
                "Helicone-Session-Path": path,
                "Helicone-Session-Name": name,
            }
        }
    if gateway == "litellm":
        return {"body": {"litellm_session_id": thread_id}}
    if gateway in ("tensorzero", "tensorzero-native"):
        if not _UUID_V7.match(thread_id):
            raise CacheError("TensorZero takes a UUIDv7 episode id; this thread id is not one")
        field = "tensorzero::episode_id" if gateway == "tensorzero" else "episode_id"
        return {"body": {field: thread_id}}
    raise CacheError(f"unknown gateway {gateway}")
