//! HTTP routes in the capability contract must declare known capabilities.

use std::path::PathBuf;

use maidan_auth::capability;

#[derive(serde::Deserialize)]
struct HttpRouteCase {
    method: String,
    path: String,
    capability: String,
}

#[test]
fn http_capability_routes_use_known_capabilities() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/http-capability-routes.json");
    let routes: Vec<HttpRouteCase> = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("http routes json");

    let known = [
        capability::WORKSPACE_READ,
        capability::WORKSPACE_WRITE,
        capability::MESSAGE_POST,
        capability::ARTIFACT_UPLOAD,
        capability::SEARCH_QUERY,
        capability::EVENT_SUBSCRIBE,
        capability::THREAD_TRANSITION,
        capability::TOKEN_ADMIN,
    ];

    for route in routes {
        assert!(
            known.contains(&route.capability.as_str()),
            "{} {} has unknown capability {}",
            route.method,
            route.path,
            route.capability
        );
    }
}
