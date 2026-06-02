//! HTTP sample routes and full map declare known capabilities.

use std::path::PathBuf;

use maidan_auth::capability;

#[derive(serde::Deserialize)]
struct HttpRouteCase {
    method: String,
    path: String,
    capability: String,
}

#[derive(serde::Deserialize)]
struct MapEntry {
    method: String,
    path: String,
    capability: String,
}

fn is_known_capability(cap: &str) -> bool {
    capability::is_known(cap) || cap == "per-tool" || cap == "per-rpc"
}

#[test]
fn http_capability_routes_use_known_capabilities() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/http-capability-routes.json");
    let routes: Vec<HttpRouteCase> = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("http routes json");

    for route in routes {
        assert!(
            is_known_capability(&route.capability),
            "{} {} has unknown capability {}",
            route.method,
            route.path,
            route.capability
        );
    }
}

#[test]
fn http_capability_map_samples_are_subset_of_full_map() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts");
    let samples: Vec<HttpRouteCase> = serde_json::from_slice(
        &std::fs::read(root.join("http-capability-routes.json"))
            .expect("read http-capability-routes.json"),
    )
    .expect("samples json");
    let map: Vec<MapEntry> = serde_json::from_slice(
        &std::fs::read(root.join("http-capability-map.json"))
            .expect("read http-capability-map.json"),
    )
    .expect("map json");

    for sample in samples {
        let normalized = sample
            .path
            .replace("{workspace_id}", "{wid}")
            .replace("/deliveries/1/", "/deliveries/{did}/");
        let found = map.iter().any(|e| {
            e.method == sample.method
                && (e.path == sample.path || e.path == normalized)
                && e.capability == sample.capability
        });
        assert!(
            found,
            "sample {} {} ({}) not in http-capability-map.json",
            sample.method, sample.path, sample.capability
        );
    }
}
