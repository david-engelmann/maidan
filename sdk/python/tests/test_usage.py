"""The usage normalizers against the shared fixtures in sdk/usage-fixtures/.

The other SDKs and the server's ledger read the same files (no server needed).
"""

import json
from pathlib import Path

import pytest

from maidan import USAGE_PROVIDERS, UsageError, normalize_usage, usd_micros

ROOT = Path(__file__).resolve().parents[2] / "usage-fixtures"


def _load(name):
    paths = sorted((ROOT / name).glob("*.json"))
    assert paths, f"no fixtures in {name}"
    return [json.loads(p.read_text()) for p in paths]


PROVIDERS = _load("providers")
CONSTRUCTED = _load("constructed")
INVALID = _load("invalid")


def _call(fixture):
    options = fixture.get("options", {})
    return normalize_usage(
        fixture["provider"],
        fixture["response"],
        model=options.get("model"),
        evidence_provider=options.get("provider"),
    )


@pytest.mark.parametrize(
    "fixture", PROVIDERS + CONSTRUCTED, ids=[f["name"] for f in PROVIDERS + CONSTRUCTED]
)
def test_normalizes_the_provider_fixture(fixture):
    got = _call(fixture)
    expected = fixture["expected"]
    assert got == {
        "model": expected["model"],
        "tokens": expected["tokens"],
        "evidence": expected["evidence"],
    }
    if "usd_micros" in expected:
        assert usd_micros(got["tokens"], fixture["price_snapshot"]) == expected["usd_micros"]


def test_every_provider_has_a_recorded_fixture():
    for provider in USAGE_PROVIDERS:
        assert any(f["provider"] == provider for f in PROVIDERS), provider
    for f in PROVIDERS:
        assert f["recorded"] in ("verbatim", "transcribed"), f["name"]
    for f in CONSTRUCTED:
        assert f["recorded"] == "constructed", f["name"]


@pytest.mark.parametrize("fixture", INVALID, ids=[f["name"] for f in INVALID])
def test_refuses_the_invalid_fixture(fixture):
    with pytest.raises(UsageError, match=fixture["expected_error"]):
        _call(fixture)


def test_prices_every_charge_fixture_as_the_ledger_does():
    charges = json.loads((ROOT / "charges.json").read_text())
    for case in charges["cases"]:
        assert usd_micros(case["tokens"], case["price_snapshot"]) == case["usd_micros"], case["name"]


def test_a_response_that_names_no_model_needs_one_passed_in():
    response = {"usage": {"input_tokens": 1, "output_tokens": 1}}
    with pytest.raises(UsageError, match="model is required"):
        normalize_usage("anthropic", response)
    assert normalize_usage("anthropic", response, model="m")["model"] == "m"


@pytest.mark.parametrize("bad", [-1, 1.5, "7", True])
def test_a_count_that_is_not_a_non_negative_integer_is_refused(bad):
    response = {"model": "m", "usage": {"prompt_tokens": bad}}
    with pytest.raises(UsageError, match="prompt_tokens must be a non-negative integer"):
        normalize_usage("openai-chat", response)


def test_an_unknown_provider_is_refused():
    with pytest.raises(UsageError, match="unknown provider"):
        normalize_usage("nope", {})


def test_the_evidence_provider_can_be_overridden_for_a_hosted_shape():
    response = {"model": "m", "usage": {"prompt_tokens": 3, "completion_tokens": 1}}
    got = normalize_usage("openai-chat", response, evidence_provider="azure.ai.openai")
    assert got["evidence"]["provider"] == "azure.ai.openai"
