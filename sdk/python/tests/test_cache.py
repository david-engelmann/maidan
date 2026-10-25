"""The cache helpers against the shared cases in sdk/cache-fixtures/ (no server)."""

import json
from pathlib import Path

import pytest

from maidan import (
    BootPrefix,
    CacheError,
    boot_prefix,
    cache_key,
    cache_key_fields,
    cached_prefix,
    cache_salt,
    gateway_session,
    routing_hints,
)

CASES = json.loads(
    (Path(__file__).resolve().parents[2] / "cache-fixtures" / "cases.json").read_text()
)


def test_the_boot_prefix_keeps_the_served_bytes_and_hashes_them():
    want = BootPrefix(**CASES["boot"])
    assert boot_prefix(CASES["boot"]["text"].encode("utf-8")) == want
    assert boot_prefix(CASES["boot"]["text"]) == want


def test_the_prefix_is_placed_with_each_providers_breakpoint():
    for case in CASES["cached_prefix"]:
        call = lambda: cached_prefix(case["provider"], case["text"], ttl=case.get("ttl"))  # noqa: E731
        if "expected_error" in case:
            with pytest.raises(CacheError, match=case["expected_error"]):
                call()
        else:
            assert call() == case["expected"], case["provider"]


def test_a_cache_key_is_per_workspace_and_group():
    for case in CASES["cache_key"]:
        assert cache_key(case["workspace_id"], case["group"]) == case["expected"]
    a, b = CASES["cache_key"]
    assert a["group"] == b["group"] and a["expected"] != b["expected"]
    with pytest.raises(CacheError):
        cache_key("", "g")


def test_the_cache_key_goes_where_each_provider_reads_it():
    for case in CASES["cache_key_fields"]:
        if "expected_error" in case:
            with pytest.raises(CacheError, match=case["expected_error"]):
                cache_key_fields(case["provider"], "K")
        else:
            assert cache_key_fields(case["provider"], "K") == case["expected"], case["provider"]


def test_the_thread_id_is_each_gateways_session_id():
    for case in CASES["gateway_session"]:
        if "expected_error" in case:
            with pytest.raises(CacheError, match=case["expected_error"]):
                gateway_session(case["gateway"], case["thread_id"])
        else:
            assert gateway_session(case["gateway"], case["thread_id"]) == case["expected"], case["gateway"]


def test_a_child_thread_is_routed_by_its_parent_on_every_router():
    for case in CASES["routing_hints"]:
        call = lambda: routing_hints(case["router"], case["thread_id"], case.get("parent_thread_id"))  # noqa: E731
        if "expected_error" in case:
            with pytest.raises(CacheError, match=case["expected_error"]):
                call()
        else:
            assert call() == case["expected"], case["router"]


def test_the_cache_salt_is_shared_inside_a_workspace_and_differs_across_workspaces_and_keys():
    for case in CASES["cache_salt"]:
        if "expected_error" in case:
            with pytest.raises(CacheError, match=case["expected_error"]):
                cache_salt(case["workspace_id"], case["salt_key"])
        else:
            assert cache_salt(case["workspace_id"], case["salt_key"]) == case["expected"]
    salts = [c["expected"]["body"]["cache_salt"] for c in CASES["cache_salt"] if "expected" in c]
    assert len(salts) == 4
    assert salts[0] != salts[1], "two workspaces share a salt"
    assert salts[0] != salts[2], "two keys give one salt"
    assert all(len(s) == 43 for s in salts)
