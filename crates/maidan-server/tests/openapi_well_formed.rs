//! The served OpenAPI document is one a client generator can use: every
//! `$ref` resolves, and every operation declares exactly the path parameters
//! its path template names. `GET /openapi.json` is what Integration.md tells
//! integrators to generate clients from, and a generator stops at the first
//! reference it cannot resolve.

use std::collections::BTreeSet;
use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, tokio::task::JoinHandle<()>, tempfile::TempDir) {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");

    let store = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().expect("tempdir");
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, server, dir)
}

async fn document() -> serde_json::Value {
    let (addr, _server, _dir) = spawn().await;
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
        .get(format!("http://{addr}/openapi.json"))
        .send()
        .await
        .expect("request")
        .json()
        .await
        .expect("json")
}

fn resolves(doc: &serde_json::Value, reference: &str) -> bool {
    let Some(pointer) = reference.strip_prefix('#') else {
        return false;
    };
    doc.pointer(pointer).is_some()
}

fn collect_refs(value: &serde_json::Value, out: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(r)) = map.get("$ref") {
                out.insert(r.clone());
            }
            map.values().for_each(|v| collect_refs(v, out));
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| collect_refs(v, out)),
        _ => {}
    }
}

#[tokio::test]
async fn every_reference_in_the_openapi_document_resolves() {
    let doc = document().await;
    let mut refs = BTreeSet::new();
    collect_refs(&doc, &mut refs);
    assert!(
        refs.len() > 50,
        "found too few references to be checking anything"
    );
    let unresolved: Vec<&String> = refs.iter().filter(|r| !resolves(&doc, r)).collect();
    assert!(
        unresolved.is_empty(),
        "{} references resolve to nothing: {unresolved:?}",
        unresolved.len()
    );
}

const METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head", "options"];

fn path_params(params: Option<&serde_json::Value>, doc: &serde_json::Value) -> BTreeSet<String> {
    params
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| match p.get("$ref").and_then(|r| r.as_str()) {
            Some(r) => doc.pointer(r.trim_start_matches('#')).cloned(),
            None => Some(p.clone()),
        })
        .filter(|p| p["in"] == "path")
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn every_operation_declares_exactly_its_path_parameters() {
    let doc = document().await;
    let paths = doc["paths"].as_object().expect("paths");
    let mut wrong = Vec::new();
    for (path, item) in paths {
        let templated: BTreeSet<String> = path
            .split('{')
            .skip(1)
            .filter_map(|s| s.split('}').next())
            .map(str::to_owned)
            .collect();
        let shared = path_params(item.get("parameters"), &doc);
        for method in METHODS {
            let Some(op) = item.get(*method) else {
                continue;
            };
            let mut declared = path_params(op.get("parameters"), &doc);
            declared.extend(shared.iter().cloned());
            if declared != templated {
                wrong.push(format!(
                    "{} {path}: template {templated:?}, declared {declared:?}",
                    method.to_uppercase()
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} operations mismatch their path template:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
