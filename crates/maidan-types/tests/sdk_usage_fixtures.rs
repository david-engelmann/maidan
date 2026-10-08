//! The four SDKs' usage normalizers and the ledger agree on the shared fixtures
//! in `sdk/usage-fixtures/`.
//!
//! Each SDK turns a fixture's provider response into the ledger's tokens. Here
//! the same response's usage object is flattened with dotted keys, the only
//! transformation, and read by `token_usage_from_genai`, the function behind
//! `POST /threads/{id}/usage/otel`. Both must give the fixture's expected
//! tokens, so neither side can drift without one of the five suites failing.

use std::path::{Path, PathBuf};

use maidan_types::{token_usage_from_genai, PriceSnapshot, TokenUsage};
use serde_json::{Map, Value};

const PROVIDERS: [&str; 9] = [
    "anthropic",
    "bedrock-converse",
    "openai-responses",
    "openai-chat",
    "gemini",
    "deepseek",
    "mistral",
    "xai",
    "vllm",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdk/usage-fixtures")
}

fn read_dir(dir: &Path) -> Vec<Value> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("{}: {err}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures in {}", dir.display());
    paths
        .iter()
        .map(|path| serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap())
        .collect()
}

fn flatten(prefix: &str, value: &Value, out: &mut Map<String, Value>) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, child, out);
            }
        }
        other => {
            out.insert(prefix.to_owned(), other.clone());
        }
    }
}

fn usage_of(response: &Value) -> &Value {
    response
        .get("usage")
        .or_else(|| response.get("usageMetadata"))
        .expect("fixture response carries usage or usageMetadata")
}

/// The ledger's provider name for a fixture: what the SDK puts in
/// `evidence.provider`, or what it would for an invalid one.
fn ledger_provider(fixture: &Value) -> String {
    if let Some(name) = fixture.pointer("/expected/evidence/provider") {
        return name.as_str().unwrap().to_owned();
    }
    match fixture["provider"].as_str().unwrap() {
        "anthropic" => "anthropic",
        "bedrock-converse" => "aws.bedrock",
        "openai-responses" | "openai-chat" => "openai",
        "gemini" => "gcp.gemini",
        "deepseek" => "deepseek",
        "mistral" => "mistral_ai",
        "xai" => "x_ai",
        "vllm" => "vllm",
        other => panic!("unknown provider {other}"),
    }
    .to_owned()
}

fn ledger_reads(fixture: &Value) -> Result<TokenUsage, String> {
    let mut attrs = Map::new();
    flatten("", usage_of(&fixture["response"]), &mut attrs);
    attrs.insert(
        "gen_ai.provider.name".into(),
        Value::String(ledger_provider(fixture)),
    );
    attrs.insert("gen_ai.response.model".into(), Value::String("m".into()));
    token_usage_from_genai(&attrs).map(|(tokens, _, _)| tokens)
}

fn assert_reads_as_expected(fixture: &Value) {
    let name = fixture["name"].as_str().unwrap();
    assert!(
        fixture["source"].as_str().unwrap().starts_with("https://"),
        "{name} names its source"
    );
    let expected: TokenUsage =
        serde_json::from_value(fixture["expected"]["tokens"].clone()).unwrap();
    let tokens = ledger_reads(fixture).unwrap_or_else(|err| panic!("{name}: {err}"));
    assert_eq!(tokens, expected, "{name}");

    if let Some(usd) = fixture["expected"].get("usd_micros") {
        let price: PriceSnapshot =
            serde_json::from_value(fixture["price_snapshot"].clone()).unwrap();
        assert_eq!(
            price.charge_usd_micros(tokens),
            Ok(usd.as_i64().unwrap()),
            "{name}"
        );
    }
}

/// `providers/` holds only responses a provider or a user published: copied
/// (`verbatim`) or retyped from an SDK's printout (`transcribed`). A shape with
/// chosen numbers goes in `constructed/`, so the per-provider check below can
/// only be met by a recorded response.
#[test]
fn the_ledger_reads_every_provider_fixture_as_the_sdks_must() {
    let fixtures = read_dir(&fixtures_dir().join("providers"));
    for fixture in &fixtures {
        let name = fixture["name"].as_str().unwrap();
        assert!(
            matches!(
                fixture["recorded"].as_str(),
                Some("verbatim" | "transcribed")
            ),
            "{name}: a provider fixture is a recorded response; a shape with chosen numbers goes in constructed/"
        );
        assert_reads_as_expected(fixture);
    }
    for provider in PROVIDERS {
        assert!(
            fixtures.iter().any(|f| f["provider"] == provider),
            "no recorded fixture for {provider}"
        );
    }
}

#[test]
fn the_ledger_reads_every_constructed_case_as_the_sdks_must() {
    for fixture in read_dir(&fixtures_dir().join("constructed")) {
        let name = fixture["name"].as_str().unwrap();
        assert_eq!(fixture["recorded"], "constructed", "{name}");
        assert_reads_as_expected(&fixture);
    }
}

#[test]
fn the_ledger_refuses_every_invalid_fixture_as_the_sdks_must() {
    for fixture in read_dir(&fixtures_dir().join("invalid")) {
        let name = fixture["name"].as_str().unwrap();
        let needle = fixture["expected_error"].as_str().unwrap();
        let err = ledger_reads(&fixture).expect_err(name);
        assert!(err.contains(needle), "{name}: {err}");
    }
}

#[test]
fn the_ledger_prices_every_charge_fixture_as_the_sdks_must() {
    let charges: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures_dir().join("charges.json")).unwrap(),
    )
    .unwrap();
    for case in charges["cases"].as_array().unwrap() {
        let tokens: TokenUsage = serde_json::from_value(case["tokens"].clone()).unwrap();
        let price: PriceSnapshot = serde_json::from_value(case["price_snapshot"].clone()).unwrap();
        assert_eq!(
            price.charge_usd_micros(tokens),
            Ok(case["usd_micros"].as_i64().unwrap()),
            "{}",
            case["name"]
        );
    }
}
