//! Ensures `PeerId` participates in the shared typed-ID conventions.

use maidan_types::PeerId;

#[test]
fn peer_id_serializes_as_uuid_string() {
    let id = PeerId(uuid::Uuid::new_v4());
    let json = serde_json::to_string(&id).expect("serialize");
    let parsed: PeerId = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, id);
}

#[test]
fn peer_id_display_matches_inner_uuid() {
    let uuid = uuid::Uuid::new_v4();
    let id = PeerId(uuid);
    assert_eq!(id.to_string(), uuid.to_string());
}
