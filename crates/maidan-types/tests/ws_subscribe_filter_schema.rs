#[test]
fn ws_subscribe_filter_schema_is_valid_json() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/ws-subscribe-filter.schema.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["title"], "Maidan WebSocket subscribe filter");
    assert!(v["properties"]["workspace_id"].is_object());
}
