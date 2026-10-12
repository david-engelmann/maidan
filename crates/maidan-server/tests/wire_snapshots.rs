//! Wire snapshots for the server's two self-description documents: the
//! A2A agent card and the OpenAPI document.
//!
//! Both are how a connected app learns what Maidan is before it holds a
//! token, and both are assembled from many places — a route added here,
//! a description edited there. Pinning the whole document makes every
//! such change one visible fixture diff in review. The documents carry
//! the build's version, which changes every release; the version fields
//! (and only they) are replaced with a placeholder before comparing, so
//! a release bump does not churn the fixtures but a renamed or dropped
//! field still fails.
//!
//! Regenerate with `MAIDAN_WRITE_SNAPSHOTS=1` when a change is intended.

use chrono::{DateTime, Utc};
use maidan_server::a2a_agent::A2aCardConfig;
use serde_json::Value;
use std::path::PathBuf;

const VERSION_PLACEHOLDER: &str = "0.0.0-snapshot";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wire")
}

fn pretty(value: &Value) -> String {
    let mut s = serde_json::to_string_pretty(value).expect("pretty");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn assert_snapshot(name: &str, value: &Value) {
    let got = pretty(value);
    let path = fixtures_dir().join(name);
    if std::env::var("MAIDAN_WRITE_SNAPSHOTS").ok().as_deref() == Some("1") {
        std::fs::create_dir_all(fixtures_dir()).expect("mkdir");
        std::fs::write(&path, &got).expect("write snapshot");
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        got, expected,
        "wire snapshot drift in {name} — rerun with MAIDAN_WRITE_SNAPSHOTS=1 if intentional"
    );
}

#[test]
fn the_a2a_agent_card_is_pinned() {
    // A configured instance: public origin and a gRPC address, so the
    // snapshot pins the GRPC interface entry as well as the HTTP ones.
    let config = A2aCardConfig {
        public_origin: Some("https://maidan.example.com".to_string()),
        grpc_public_addr: Some("grpc.maidan.example.com:443".to_string()),
        issued_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("fixed unix seconds"),
    };
    let mut card = serde_json::to_value(config.card()).expect("card serializes");
    card["version"] = Value::from(VERSION_PLACEHOLDER);
    assert_snapshot("a2a_agent_card.json", &card);
}

#[test]
fn the_openapi_document_is_pinned() {
    let mut doc =
        serde_json::to_value(maidan_server::openapi::document()).expect("document serializes");
    doc["info"]["version"] = Value::from(VERSION_PLACEHOLDER);
    assert_snapshot("openapi.json", &doc);
}
