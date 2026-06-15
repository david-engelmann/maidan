//! Capability vocabulary + `AuthContext` authorization matrix + constant-time
//! hash comparison. Pure logic — no store, no async.

use maidan_auth::capability::{
    self, ARTIFACT_UPLOAD, EVENT_SUBSCRIBE, FEDERATION_ADMIN, FEDERATION_INGEST, MESSAGE_POST,
    SEARCH_QUERY, THREAD_TRANSITION, TOKEN_ADMIN, WORKSPACE_READ, WORKSPACE_WRITE,
};
use maidan_auth::token::hashes_equal;
use maidan_auth::AuthContext;
use maidan_types::{ApiTokenId, AppInstallationId, MemberId, WorkspaceId};

fn member() -> MemberId {
    MemberId(uuid::Uuid::new_v4())
}

fn workspace() -> WorkspaceId {
    WorkspaceId(uuid::Uuid::new_v4())
}

// --- capability vocabulary -------------------------------------------------

#[test]
fn every_named_capability_is_known() {
    for cap in [
        WORKSPACE_READ,
        WORKSPACE_WRITE,
        MESSAGE_POST,
        THREAD_TRANSITION,
        ARTIFACT_UPLOAD,
        SEARCH_QUERY,
        EVENT_SUBSCRIBE,
        TOKEN_ADMIN,
        FEDERATION_INGEST,
        FEDERATION_ADMIN,
    ] {
        assert!(
            capability::is_known(cap),
            "{cap} should be a known capability"
        );
    }
}

#[test]
fn unknown_capabilities_are_rejected() {
    assert!(!capability::is_known("workspace:admin"));
    assert!(!capability::is_known("WORKSPACE_READ"));
    assert!(!capability::is_known(""));
    assert!(!capability::is_known("message:post ")); // trailing space is not normalized away
}

#[test]
fn default_minted_set_is_a_known_subset() {
    let minted = capability::default_minted();
    assert!(capability::validate_list(&minted).is_ok());
    assert!(minted.iter().any(|c| c == WORKSPACE_READ));
    assert!(minted.iter().any(|c| c == WORKSPACE_WRITE));
    // The default mint is deliberately read/write/subscribe/search — never admin.
    assert!(!minted.iter().any(|c| c == TOKEN_ADMIN));
    assert!(!minted.iter().any(|c| c == FEDERATION_ADMIN));
}

#[test]
fn validate_list_flags_the_first_unknown_capability() {
    let caps = vec![WORKSPACE_READ.to_string(), "bogus:cap".to_string()];
    let err = capability::validate_list(&caps).expect_err("unknown cap must error");
    assert!(err.contains("bogus:cap"));
    assert!(capability::validate_list(&[]).is_ok());
}

#[test]
fn validate_subset_requires_each_requested_capability_to_be_granted() {
    let granted = vec![WORKSPACE_READ.to_string(), MESSAGE_POST.to_string()];

    // A subset of the grant is fine.
    assert!(capability::validate_subset(&granted, &[WORKSPACE_READ.to_string()]).is_ok());
    assert!(capability::validate_subset(&granted, &granted).is_ok());

    // Requesting beyond the grant is rejected even though the capability is known.
    let err = capability::validate_subset(&granted, &[WORKSPACE_WRITE.to_string()])
        .expect_err("escalation must error");
    assert!(err.contains("exceeds app installation grant"));
}

#[test]
fn validate_subset_rejects_unknown_before_checking_the_grant() {
    let granted = vec![WORKSPACE_READ.to_string()];
    let err = capability::validate_subset(&granted, &["nope:cap".to_string()])
        .expect_err("unknown requested cap must error");
    assert!(err.contains("unknown capability"));
}

// --- AuthContext authorization matrix --------------------------------------

#[test]
fn token_context_grants_only_its_listed_capabilities() {
    let ctx = AuthContext::from_token(
        ApiTokenId(uuid::Uuid::new_v4()),
        member(),
        workspace(),
        vec![WORKSPACE_READ.to_string(), SEARCH_QUERY.to_string()],
    );
    assert!(ctx.has_capability(WORKSPACE_READ));
    assert!(ctx.has_capability(SEARCH_QUERY));
    assert!(!ctx.has_capability(WORKSPACE_WRITE));
    assert!(ctx.require_capability(WORKSPACE_READ).is_ok());
    let err = ctx
        .require_capability(WORKSPACE_WRITE)
        .expect_err("missing cap must be forbidden");
    assert!(matches!(err, maidan_auth::AuthError::Forbidden(_)));
    assert!(err.to_string().contains(WORKSPACE_WRITE));
    assert!(!ctx.bypass);
    assert!(ctx.app_installation_id.is_none());
}

#[test]
fn app_token_context_carries_installation_id() {
    let installation = AppInstallationId(uuid::Uuid::new_v4());
    let ctx = AuthContext::from_app_token(
        ApiTokenId(uuid::Uuid::new_v4()),
        member(),
        workspace(),
        installation,
        vec![MESSAGE_POST.to_string()],
    );
    assert_eq!(ctx.app_installation_id, Some(installation));
    assert!(ctx.has_capability(MESSAGE_POST));
    assert!(!ctx.has_capability(TOKEN_ADMIN));
}

#[test]
fn session_context_has_no_token_id() {
    let ctx = AuthContext::from_session(member(), workspace(), vec![WORKSPACE_READ.to_string()]);
    assert!(ctx.token_id.is_none());
    assert!(ctx.has_capability(WORKSPACE_READ));
    assert!(!ctx.has_capability(MESSAGE_POST));
}

#[test]
fn bypass_context_satisfies_every_capability_and_workspace() {
    let ctx = AuthContext::bypass();
    assert!(ctx.bypass);
    // bypass short-circuits every capability check, including unknown strings.
    assert!(ctx.has_capability(TOKEN_ADMIN));
    assert!(ctx.has_capability("anything:at:all"));
    assert!(ctx.require_capability(FEDERATION_ADMIN).is_ok());
    // ...and any workspace.
    assert!(ctx.ensure_workspace(workspace()).is_ok());
}

#[test]
fn ensure_workspace_is_scoped_to_the_tokens_workspace() {
    let ws = workspace();
    let other = workspace();
    let ctx = AuthContext::from_token(
        ApiTokenId(uuid::Uuid::new_v4()),
        member(),
        ws,
        capability::default_minted(),
    );
    assert!(ctx.ensure_workspace(ws).is_ok());
    let err = ctx
        .ensure_workspace(other)
        .expect_err("cross-workspace access must be forbidden");
    assert!(matches!(err, maidan_auth::AuthError::Forbidden(_)));
}

// --- constant-time hash comparison -----------------------------------------

#[test]
fn hashes_equal_matches_only_identical_digests() {
    let a = maidan_auth::hash_secret("maid_alpha");
    let a_again = maidan_auth::hash_secret("maid_alpha");
    let b = maidan_auth::hash_secret("maid_beta");
    assert!(hashes_equal(&a, &a_again));
    assert!(!hashes_equal(&a, &b));
}

#[test]
fn hashes_equal_rejects_length_mismatch_without_panic() {
    // The length guard runs before the constant-time compare; a short candidate
    // must never index past the stored digest.
    assert!(!hashes_equal(&"a".repeat(64), "a"));
    assert!(!hashes_equal("", &"a".repeat(64)));
    assert!(hashes_equal("", "")); // equal length (zero) — trivially equal
    assert!(hashes_equal("deadbeef", "deadbeef"));
}

#[test]
fn hash_secret_is_deterministic_lowercase_hex() {
    let h = maidan_auth::hash_secret("maid_determinism");
    assert_eq!(h.len(), 64);
    assert!(h
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_eq!(h, maidan_auth::hash_secret("maid_determinism"));
    assert_ne!(h, maidan_auth::hash_secret("maid_determinisn"));
}
