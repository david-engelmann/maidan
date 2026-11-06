"""Boundary input gates for the Python SDK cache and usage helpers."""

from __future__ import annotations

import hashlib
import pytest

from maidan.cache import (
    CacheError,
    cache_key,
    cache_key_fields,
    cache_salt,
    cached_prefix,
    gateway_session,
    require_text,
    routing_hints,
)
from maidan.usage import UsageError, normalize_usage


@pytest.mark.parametrize("bad", ["", "   ", "\t", None, 12, True])
def test_require_text_refuses_blank_and_non_string(bad):
    with pytest.raises(CacheError, match="label is required"):
        require_text(bad, "label")


def test_blank_or_non_string_cache_inputs_are_refused():
    for bad in ("", "   ", None, 12):
        with pytest.raises(CacheError, match="is required"):
            cache_key(bad, "g")  # type: ignore[arg-type]
        with pytest.raises(CacheError, match="is required"):
            cache_key("ws", bad)  # type: ignore[arg-type]
    with pytest.raises(CacheError, match="provider is required"):
        cached_prefix("   ", "hi")
    with pytest.raises(CacheError, match="cache key is required"):
        cache_key_fields("openai-chat", "  ")
    with pytest.raises(CacheError, match="thread id is required"):
        gateway_session("openrouter", "\t")
    with pytest.raises(CacheError, match="thread id is required"):
        routing_hints("dynamo", "  ")
    with pytest.raises(CacheError, match="workspace id is required"):
        cache_salt("  ", "k" * 32)


def test_blank_providers_and_prototype_path_keys_are_refused():
    with pytest.raises(UsageError, match="provider is required"):
        normalize_usage("  ", {"model": "m", "usage": {}})
    with pytest.raises(UsageError, match="provider is required"):
        normalize_usage(None, {"model": "m", "usage": {}})  # type: ignore[arg-type]
    # Own-key walk: decoy keys named like prototype edges are ignored.
    nested = {
        "model": "m",
        "usage": {
            "prompt_tokens": 3,
            "completion_tokens": 1,
            "prompt_tokens_details": {"__proto__": {"cached_tokens": 99}},
        },
    }
    got = normalize_usage("openai-chat", nested)
    assert got["tokens"]["cache_read"] == 0
    with pytest.raises(UsageError, match="evidence provider is required"):
        normalize_usage("openai-chat", {"model": "m", "usage": {"prompt_tokens": 1, "completion_tokens": 1}}, evidence_provider="  ")


def test_padded_cache_inputs_keep_original_bytes():
    got = cache_key(" ws", "group ")
    digest = hashlib.sha256(b" ws\ngroup ").hexdigest()
    assert got == f"maidan-{digest[:32]}"
