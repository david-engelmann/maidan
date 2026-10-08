//! The usage normalizers against the shared fixtures in `sdk/usage-fixtures/`,
//! which the other SDKs and the server's ledger read too (no server needed).

use std::path::PathBuf;

use maidan::{
    normalize_usage, usd_micros, NormalizedUsage, PriceSnapshot, TokenUsage, UsageOptions,
    USAGE_PROVIDERS,
};
use serde_json::{json, Value};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../usage-fixtures")
}

fn load(dir: &str) -> Vec<Value> {
    let mut paths: Vec<_> = std::fs::read_dir(root().join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures in {dir}");
    paths
        .iter()
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap())
        .collect()
}

fn options(fixture: &Value) -> UsageOptions {
    let get = |key: &str| {
        fixture
            .pointer(&format!("/options/{key}"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    UsageOptions {
        model: get("model"),
        provider: get("provider"),
    }
}

fn normalize(fixture: &Value) -> Result<NormalizedUsage, maidan::UsageError> {
    normalize_usage(
        fixture["provider"].as_str().unwrap(),
        &fixture["response"],
        &options(fixture),
    )
}

#[test]
fn every_provider_fixture_normalizes_to_its_expected_report() {
    let fixtures = load("providers");
    let constructed = load("constructed");
    for fixture in &fixtures {
        let name = fixture["name"].as_str().unwrap();
        assert!(
            matches!(
                fixture["recorded"].as_str(),
                Some("verbatim" | "transcribed")
            ),
            "{name}: a provider fixture is a recorded response"
        );
    }
    for fixture in &constructed {
        assert_eq!(fixture["recorded"], "constructed", "{}", fixture["name"]);
    }
    for fixture in fixtures.iter().chain(&constructed) {
        let name = fixture["name"].as_str().unwrap();
        let got = normalize(fixture).unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = &fixture["expected"];
        let want = NormalizedUsage {
            model: expected["model"].as_str().unwrap().to_owned(),
            tokens: serde_json::from_value(expected["tokens"].clone()).unwrap(),
            evidence: serde_json::from_value(expected["evidence"].clone()).unwrap(),
        };
        assert_eq!(got, want, "{name}");
        if let Some(usd) = expected.get("usd_micros") {
            let price: PriceSnapshot =
                serde_json::from_value(fixture["price_snapshot"].clone()).unwrap();
            assert_eq!(
                usd_micros(&got.tokens, &price),
                Ok(usd.as_i64().unwrap()),
                "{name}"
            );
        }
    }
    for provider in USAGE_PROVIDERS {
        assert!(
            fixtures.iter().any(|f| f["provider"] == provider),
            "no recorded fixture for {provider}"
        );
    }
}

#[test]
fn every_invalid_fixture_is_refused() {
    for fixture in load("invalid") {
        let name = fixture["name"].as_str().unwrap();
        let needle = fixture["expected_error"].as_str().unwrap();
        let err = normalize(&fixture).expect_err(name);
        assert!(err.0.contains(needle), "{name}: {err}");
    }
}

#[test]
fn every_charge_fixture_prices_as_the_ledger_does() {
    let charges: Value =
        serde_json::from_str(&std::fs::read_to_string(root().join("charges.json")).unwrap())
            .unwrap();
    for case in charges["cases"].as_array().unwrap() {
        let tokens: TokenUsage = serde_json::from_value(case["tokens"].clone()).unwrap();
        let price: PriceSnapshot = serde_json::from_value(case["price_snapshot"].clone()).unwrap();
        assert_eq!(
            usd_micros(&tokens, &price),
            Ok(case["usd_micros"].as_i64().unwrap()),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn a_response_that_names_no_model_needs_one_passed_in() {
    let response = json!({"usage": {"input_tokens": 1, "output_tokens": 1}});
    let err = normalize_usage("anthropic", &response, &UsageOptions::default()).unwrap_err();
    assert!(err.0.contains("model is required"), "{err}");
    let with = UsageOptions {
        model: Some("m".into()),
        ..UsageOptions::default()
    };
    assert_eq!(
        normalize_usage("anthropic", &response, &with)
            .unwrap()
            .model,
        "m"
    );
}

#[test]
fn bad_counts_and_unknown_providers_are_refused() {
    let none = UsageOptions::default();
    assert!(normalize_usage("nope", &json!({}), &none)
        .unwrap_err()
        .0
        .contains("unknown provider"));
    for bad in [json!(-1), json!(1.5), json!("7"), json!(true)] {
        let response = json!({"model": "m", "usage": {"prompt_tokens": bad}});
        let err = normalize_usage("openai-chat", &response, &none).unwrap_err();
        assert!(
            err.0
                .contains("prompt_tokens must be a non-negative integer"),
            "{err}"
        );
    }
}

#[test]
fn the_evidence_provider_can_be_overridden_for_a_hosted_shape() {
    let response = json!({"model": "m", "usage": {"prompt_tokens": 3, "completion_tokens": 1}});
    let azure = UsageOptions {
        provider: Some("azure.ai.openai".into()),
        ..UsageOptions::default()
    };
    let got = normalize_usage("openai-chat", &response, &azure).unwrap();
    assert_eq!(got.evidence.provider, "azure.ai.openai");
}
