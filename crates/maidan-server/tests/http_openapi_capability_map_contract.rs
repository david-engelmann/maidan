//! OpenAPI bearer operations must match `contracts/http-capability-map.json`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use maidan_auth::capability;
use maidan_server::openapi::ApiDoc;
use utoipa::openapi::path::{Operation, PathItemType};
use utoipa::OpenApi;

#[derive(Debug, serde::Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct RouteKey {
    method: String,
    path: String,
}

#[derive(Debug, serde::Deserialize)]
struct MapEntry {
    method: String,
    path: String,
    capability: String,
    surface: String,
}

fn load_map() -> Vec<MapEntry> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/http-capability-map.json");
    serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("http capability map json")
}

fn operation_has_bearer(op: &Operation) -> bool {
    let Ok(value) = serde_json::to_value(op) else {
        return false;
    };
    value
        .get("security")
        .and_then(|s| s.as_array())
        .is_some_and(|requirements| {
            requirements.iter().any(|req| {
                req.as_object()
                    .is_some_and(|obj| obj.contains_key("bearerAuth"))
            })
        })
}

fn path_item_type_method(item_type: &PathItemType) -> String {
    match item_type {
        PathItemType::Get => "GET",
        PathItemType::Post => "POST",
        PathItemType::Put => "PUT",
        PathItemType::Patch => "PATCH",
        PathItemType::Delete => "DELETE",
        PathItemType::Options => "OPTIONS",
        PathItemType::Head => "HEAD",
        PathItemType::Trace => "TRACE",
        PathItemType::Connect => "CONNECT",
    }
    .to_string()
}

fn collect_openapi_bearer_routes() -> BTreeSet<RouteKey> {
    let doc = ApiDoc::openapi();
    let mut out = BTreeSet::new();
    for (path, item) in doc.paths.paths.iter() {
        for (item_type, op) in item.operations.iter() {
            if !operation_has_bearer(op) {
                continue;
            }
            out.insert(RouteKey {
                method: path_item_type_method(item_type),
                path: path.clone(),
            });
        }
    }
    out
}

fn collect_map_http_routes(map: &[MapEntry]) -> BTreeSet<RouteKey> {
    map.iter()
        .filter(|e| e.surface == "http")
        .map(|e| RouteKey {
            method: e.method.clone(),
            path: e.path.clone(),
        })
        .collect()
}

fn is_known_capability(cap: &str) -> bool {
    capability::is_known(cap) || cap == "per-tool" || cap == "per-rpc"
}

#[test]
fn http_capability_map_entries_use_known_capabilities() {
    for entry in load_map() {
        assert!(
            is_known_capability(&entry.capability),
            "{} {} capability {}",
            entry.method,
            entry.path,
            entry.capability
        );
    }
}

#[test]
fn openapi_includes_cluster_77_documented_routes() {
    let doc = ApiDoc::openapi();
    for key in [
        "/workspaces/{wid}/context",
        "/workspaces/{wid}/automation/dlq",
        "/artifacts/multipart",
    ] {
        assert!(
            doc.paths.paths.contains_key(key),
            "missing OpenAPI path {key}"
        );
    }
}

#[test]
fn openapi_bearer_operations_match_http_capability_map() {
    let map = load_map();
    let openapi = collect_openapi_bearer_routes();
    let http_map = collect_map_http_routes(&map);

    let missing_from_map: Vec<_> = openapi.difference(&http_map).collect();
    let extra_in_map: Vec<_> = http_map.difference(&openapi).collect();

    assert!(
        missing_from_map.is_empty(),
        "OpenAPI bearer ops missing from http-capability-map (surface=http): {missing_from_map:?}"
    );
    assert!(
        extra_in_map.is_empty(),
        "http-capability-map (surface=http) has entries not in OpenAPI bearer ops: {extra_in_map:?}"
    );
}
