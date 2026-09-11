//! The egress SecretBroker (Cluster 371.4, Wave 2 #19). Proves the security
//! property: a `secret://<name>` ref in an outbound payload is substituted with
//! the real value ONLY when the target host is allowlisted — otherwise it's left
//! as the literal placeholder (never leaked). Exercises the async resolve +
//! decrypt path directly (the webhook worker calls the same fn at send time).

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_auth::encrypt_peer_secret;
use maidan_server::secret_broker::parse_allowlist;
use maidan_server::{secret_broker, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewMember, NewSecret, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

async fn state_with_key(key: Arc<[u8; 32]>) -> (AppState, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, Some(key)),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    (state, store)
}

#[tokio::test]
async fn broker_substitutes_only_for_allowlisted_hosts() {
    let key: Arc<[u8; 32]> = Arc::new([3u8; 32]);
    let (state, store) = state_with_key(key.clone()).await;

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    store
        .create_secret(NewSecret {
            workspace_id: ws.id,
            name: "api-key".into(),
            value_ciphertext: encrypt_peer_secret("TOPSECRET", &key).unwrap(),
            created_by: member.id,
        })
        .await
        .unwrap();

    let body = r#"{"auth":"Bearer secret://api-key","other":"secret://missing"}"#;
    let allow = parse_allowlist("hooks.example.com");

    // Allowlisted host → the known secret is substituted; an unknown ref stays literal.
    let subbed =
        secret_broker::substitute_with(&state, ws.id, "https://hooks.example.com/x", body, &allow)
            .await;
    assert!(
        subbed.contains("Bearer TOPSECRET"),
        "known secret substituted"
    );
    assert!(
        subbed.contains("secret://missing"),
        "unknown ref left literal"
    );
    assert!(!subbed.contains("secret://api-key"));

    // Non-allowlisted host → the payload is unchanged, the secret NEVER leaks.
    let untouched =
        secret_broker::substitute_with(&state, ws.id, "https://evil.example.com/x", body, &allow)
            .await;
    assert_eq!(
        untouched, body,
        "non-allowlisted host gets the literal refs"
    );
    assert!(!untouched.contains("TOPSECRET"));
}
