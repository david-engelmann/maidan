"""Keeping Maidan's bytes in the provider's cache.

The boot prefix with a cache breakpoint, one cache key per shared-prefix
group, and the thread id as a gateway session id. Pure functions; the
TypeScript, Go and Rust SDKs give the same output for the same input
(``docs/Harness Caching.md``).
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import re
from dataclasses import dataclass
from typing import Any, Union

__all__ = [
    "BootPrefix",
    "CacheError",
    "MIN_SALT_KEY_BYTES",
    "boot_prefix",
    "cache_key",
    "cache_key_fields",
    "cached_prefix",
    "cache_salt",
    "gateway_session",
    "routing_hints",
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


_ONE_KEY_ROUTERS = {
    "dynamo": ("X-Dynamo-Session-ID", None),
    "llm-d": ("x-session-id", None),
    "aibrix": ("x-aibrix-session-key", 256),
    "sglang": ("X-SMG-Routing-Key", 128),
}


def routing_hints(router: str, thread_id: str, parent_thread_id: str | None = None) -> dict:
    """Affinity headers for a self-hosted KV-aware router.

    Every router keys a child by its parent, so a child lands where its parent's
    prefix is warm: Dynamo, llm-d, AIBrix and SGLang all get the parent's thread
    as their one affinity key when there is one. Dynamo's docs say affinity uses
    ``X-Dynamo-Session-ID`` alone (with ``--router-session-affinity-ttl-secs`` set)
    and ``X-Dynamo-Parent-Session-ID`` is trace metadata, so a child sends its
    parent's thread as the session and no parent header (its own id is not a
    parent). AIBrix also gets ``routing-strategy: session-affinity``. SGLang caps
    the key at 128 bytes and AIBrix at 256; a longer key is an error. Hints only:
    no router accepts a prefix hash from a client.
    """
    if not thread_id:
        raise CacheError("thread id is required")
    parent = parent_thread_id or None
    spec = _ONE_KEY_ROUTERS.get(router)
    if spec is None:
        raise CacheError(f"unknown router {router}")
    name, limit = spec
    key = parent or thread_id
    if limit is not None and len(key.encode("utf-8")) > limit:
        raise CacheError(f"routing key is longer than {limit} bytes")
    headers = {name: key}
    if router == "aibrix":
        headers["routing-strategy"] = "session-affinity"
    return {"headers": headers}


MIN_SALT_KEY_BYTES = 32
"""The shortest ``salt_key`` :func:`cache_salt` accepts, in bytes."""


def cache_salt(workspace_id: str, salt_key: Union[str, bytes]) -> dict:
    """vLLM's ``cache_salt`` for one workspace: HMAC-SHA256(salt_key,
    "cache_salt\n" + workspace_id) as unpadded base64url (43 characters), so it
    cannot be computed from the workspace id alone. The key must be at least 32
    bytes. Set the salt from a trusted server-side component that holds the key,
    never from an end client. It is a cache partition, not an auth token.
    """
    if not workspace_id:
        raise CacheError("workspace id is required")
    key = salt_key.encode("utf-8") if isinstance(salt_key, str) else bytes(salt_key)
    if len(key) < MIN_SALT_KEY_BYTES:
        raise CacheError(f"salt key must be at least {MIN_SALT_KEY_BYTES} bytes")
    mac = hmac.new(key, f"cache_salt\n{workspace_id}".encode("utf-8"), hashlib.sha256).digest()
    return {"body": {"cache_salt": base64.urlsafe_b64encode(mac).rstrip(b"=").decode("ascii")}}
