//! Integration tests for peer URL and name validation rules.

use maidan_a2a::{validate_base_url, validate_peer_name, FederationError, NewPeer};
use maidan_types::WorkspaceId;

#[test]
fn peer_name_rejects_only_whitespace() {
    validate_peer_name("   ").expect_err("whitespace name");
}

#[test]
fn base_url_accepts_http_for_local_dev() {
    validate_base_url("http://127.0.0.1:8080").expect("local http");
}

#[test]
fn new_peer_validate_composes_name_and_url_checks() {
    let peer = NewPeer {
        workspace_id: WorkspaceId(uuid::Uuid::new_v4()),
        name: "upstream".into(),
        base_url: "ftp://wrong.scheme".into(),
    };
    let err = peer.validate().unwrap_err();
    assert!(matches!(err, FederationError::InvalidInput(_)));
}
