"""Provider usage objects -> the ledger's ``report_usage`` shape.

The ledger's ``input`` is uncached input on every provider, and cache writes
are two tiers, 5-minute and 1-hour. Providers disagree on both, so each reader
below says where its numbers come from. The TypeScript, Go and Rust SDKs and
the server's own reader agree on the fixtures in ``sdk/usage-fixtures/``.
"""

from __future__ import annotations

from typing import Any, Callable

__all__ = ["USAGE_PROVIDERS", "UsageError", "normalize_usage", "usd_micros"]

_I64_MAX = 2**63 - 1


class UsageError(ValueError):
    """A usage object that cannot be read into the ledger's shape."""


def _at(obj: Any, path: str) -> Any:
    value = obj
    for key in path.split("."):
        if not isinstance(value, dict):
            return None
        value = value.get(key)
    return value


def _count(obj: Any, path: str) -> int:
    value = _at(obj, path)
    if value is None:
        return 0
    # bool is an int subclass in Python; JSON true is not a count.
    if isinstance(value, bool) or not isinstance(value, int) or value < 0 or value > _I64_MAX:
        raise UsageError(f"{path} must be a non-negative integer")
    return value


def _text(obj: Any, path: str) -> str | None:
    value = _at(obj, path)
    if isinstance(value, str) and value.strip():
        return value.strip()
    return None


def _add(*parts: int) -> int:
    total = sum(parts)
    if total > _I64_MAX:
        raise UsageError("token total overflow")
    return total


def _uncached(total: int, *cached: int) -> int:
    """OpenAI, Gemini, Mistral, xAI and vLLM count cached tokens inside the total."""
    rest = total - _add(*cached)
    if rest < 0:
        raise UsageError("input_tokens smaller than the cache tiers it includes")
    return rest


def _block(response: dict, key: str) -> dict:
    value = response.get(key)
    if not isinstance(value, dict):
        raise UsageError(f"response has no {key} object")
    return value


def _tokens(input: int, output: int, read: int, five: int = 0, hour: int = 0) -> dict:
    return {
        "input": input,
        "output": output,
        "cache_read": read,
        "cache_write_5m": five,
        "cache_write_1h": hour,
    }


def _bedrock_details(usage: dict) -> tuple[int, int]:
    """Bedrock Converse splits its writes by TTL in ``cacheDetails``."""
    details = usage.get("cacheDetails")
    if details is None:
        return 0, 0
    if not isinstance(details, list):
        raise UsageError("cacheDetails must be an array")
    five = hour = 0
    for detail in details:
        if not isinstance(detail, dict):
            raise UsageError("cacheDetails entries must be objects")
        tokens = _count(detail, "inputTokens")
        if detail.get("ttl") == "5m":
            five = _add(five, tokens)
        elif detail.get("ttl") == "1h":
            hour = _add(hour, tokens)
        else:
            raise UsageError("cacheDetails ttl must be 5m or 1h")
    return five, hour


def _chat(usage: dict, reasoning_is_extra: bool = False) -> dict:
    read = _count(usage, "prompt_tokens_details.cached_tokens")
    write = _count(usage, "prompt_tokens_details.cache_write_tokens")
    reasoning = (
        _count(usage, "completion_tokens_details.reasoning_tokens") if reasoning_is_extra else 0
    )
    return _tokens(
        _uncached(_count(usage, "prompt_tokens"), read, write),
        _add(_count(usage, "completion_tokens"), reasoning),
        read,
        write,
    )


def _responses(usage: dict, reasoning_is_extra: bool = False) -> dict:
    read = _count(usage, "input_tokens_details.cached_tokens")
    write = _count(usage, "input_tokens_details.cache_write_tokens")
    reasoning = _count(usage, "output_tokens_details.reasoning_tokens") if reasoning_is_extra else 0
    return _tokens(
        _uncached(_count(usage, "input_tokens"), read, write),
        _add(_count(usage, "output_tokens"), reasoning),
        read,
        write,
    )


def _anthropic(r: dict) -> dict:
    u = _block(r, "usage")
    five = _count(u, "cache_creation.ephemeral_5m_input_tokens")
    hour = _count(u, "cache_creation.ephemeral_1h_input_tokens")
    if five == 0 and hour == 0:
        five = _count(u, "cache_creation_input_tokens")
    return {
        "tokens": _tokens(
            _count(u, "input_tokens"),
            _count(u, "output_tokens"),
            _count(u, "cache_read_input_tokens"),
            five,
            hour,
        ),
        "model": _text(r, "model"),
        "service_tier": _text(u, "service_tier"),
        "cache_miss_reason": _text(r, "diagnostics.cache_miss_reason.type"),
    }


def _bedrock(r: dict) -> dict:
    u = _block(r, "usage")
    five, hour = _bedrock_details(u)
    if five == 0 and hour == 0:
        five = _count(u, "cacheWriteInputTokens")
    return {
        "tokens": _tokens(
            _count(u, "inputTokens"),
            _count(u, "outputTokens"),
            _count(u, "cacheReadInputTokens"),
            five,
            hour,
        )
    }


def _openai_responses(r: dict) -> dict:
    return {
        "tokens": _responses(_block(r, "usage")),
        "model": _text(r, "model"),
        "service_tier": _text(r, "service_tier"),
    }


def _openai_chat(r: dict) -> dict:
    return {
        "tokens": _chat(_block(r, "usage")),
        "model": _text(r, "model"),
        "service_tier": _text(r, "service_tier"),
    }


def _gemini(r: dict) -> dict:
    u = _block(r, "usageMetadata")
    read = _count(u, "cachedContentTokenCount")
    return {
        "tokens": _tokens(
            _uncached(_count(u, "promptTokenCount"), read),
            _add(_count(u, "candidatesTokenCount"), _count(u, "thoughtsTokenCount")),
            read,
        ),
        "model": _text(r, "modelVersion"),
        "service_tier": _text(u, "serviceTier"),
    }


def _deepseek(r: dict) -> dict:
    u = _block(r, "usage")
    if u.get("prompt_cache_miss_tokens") is None:
        raise UsageError("prompt_cache_miss_tokens is required")
    return {
        "tokens": _tokens(
            _count(u, "prompt_cache_miss_tokens"),
            _count(u, "completion_tokens"),
            _count(u, "prompt_cache_hit_tokens"),
        ),
        "model": _text(r, "model"),
    }


def _plain_chat(r: dict) -> dict:
    return {"tokens": _chat(_block(r, "usage")), "model": _text(r, "model")}


def _xai(r: dict) -> dict:
    u = _block(r, "usage")
    if "input_tokens" in u:
        tokens = _responses(u, reasoning_is_extra=True)
    else:
        tokens = _chat(u, reasoning_is_extra=True)
    return {"tokens": tokens, "model": _text(r, "model")}


_READERS: dict[str, tuple[str, Callable[[dict], dict]]] = {
    "anthropic": ("anthropic", _anthropic),
    "bedrock-converse": ("aws.bedrock", _bedrock),
    "openai-responses": ("openai", _openai_responses),
    "openai-chat": ("openai", _openai_chat),
    "gemini": ("gcp.gemini", _gemini),
    "deepseek": ("deepseek", _deepseek),
    "mistral": ("mistral_ai", _plain_chat),
    "xai": ("x_ai", _xai),
    "vllm": ("vllm", _plain_chat),
}

USAGE_PROVIDERS: tuple[str, ...] = tuple(_READERS)


def normalize_usage(
    provider: str,
    response: dict,
    *,
    model: str | None = None,
    evidence_provider: str | None = None,
) -> dict:
    """Turn one provider response into ``{"model", "tokens", "evidence"}``.

    That is the economic part of a ``report_usage`` body. ``model`` names the
    model when the response does not (Bedrock Converse); ``evidence_provider``
    overrides the evidence provider name (a Chat Completions shape served by
    Azure).
    """
    if provider not in _READERS:
        raise UsageError(f"unknown provider {provider}")
    if not isinstance(response, dict):
        raise UsageError("response must be a JSON object")
    name, read = _READERS[provider]
    got = read(response)
    chosen = (model if model is not None else got.get("model") or "").strip()
    if not chosen:
        raise UsageError("model is required: the response names none, so pass model=")
    evidence: dict[str, str] = {"provider": evidence_provider or name}
    if got.get("service_tier"):
        evidence["service_tier"] = got["service_tier"]
    if got.get("cache_miss_reason"):
        evidence["cache_miss_reason"] = got["cache_miss_reason"]
    return {"model": chosen, "tokens": got["tokens"], "evidence": evidence}


_TIERS = (
    ("input", "input_usd_micros_per_million"),
    ("output", "output_usd_micros_per_million"),
    ("cache_read", "cache_read_usd_micros_per_million"),
    ("cache_write_5m", "cache_write_5m_usd_micros_per_million"),
    ("cache_write_1h", "cache_write_1h_usd_micros_per_million"),
)


def usd_micros(tokens: dict, price_snapshot: dict) -> int:
    """``usd_micros`` as the ledger checks it: ``ceil(sum(tokens x rate) / 1e6)``."""
    total = 0
    for tier, rate in _TIERS:
        total += _count(tokens, tier) * _count(price_snapshot, rate)
    usd = (total + 999_999) // 1_000_000
    if usd > _I64_MAX:
        raise UsageError("usd_micros overflow")
    return usd
