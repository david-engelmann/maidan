#[test]
fn ws_subscribe_filter_schema_is_valid_json() {
    let contracts = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts");
    let path = contracts.join("ws-subscribe-filter.schema.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["title"], "Maidan WebSocket subscribe filter");
    assert_eq!(
        v["$id"],
        "https://maidan.dev/schemas/ws-subscribe-filter.json"
    );
    assert_eq!(v["x-maidan-required-capability"], "event:subscribe");
    let event_kinds_contract = v["x-maidan-event-kinds-contract"]
        .as_str()
        .expect("event-kind contract link");
    let kinds: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(contracts.join(event_kinds_contract))
            .expect("linked event-kind contract"),
    )
    .expect("event-kind contract JSON");
    assert!(kinds.as_array().is_some_and(|values| !values.is_empty()));
    assert!(v["properties"]["workspace_id"].is_object());
}
