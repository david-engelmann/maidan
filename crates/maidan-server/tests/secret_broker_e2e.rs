//! The egress SecretBroker. Proves the security property: a `secret://<name>`
//! ref in an outbound payload is substituted with the real value ONLY when the
//! target host is on the sending workspace's secret-egress allowlist (and inside
//! the instance ceiling, when one is set), and only from that workspace's own
//! secrets — otherwise it's left as the literal placeholder. Exercises the
//! async resolve + decrypt path every egress worker calls at send time.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_auth::encrypt_peer_secret;
use maidan_server::{secret_broker, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewMember, NewSecret, NewSecretEgressHost, NewWorkspace, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

const KEY: [u8; 32] = [3u8; 32];

async fn broker_state(ceiling: Option<Vec<String>>) -> (AppState, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
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
        FederationRuntime::new(true, Some(Arc::new(KEY))),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    if let Some(ceiling) = ceiling {
        state.mcp.set_secret_egress_ceiling(ceiling);
    }
    (state, store)
}

async fn workspace_with_secret(
    store: &dyn Store,
    name: &str,
    secret: &str,
    value: &str,
) -> WorkspaceId {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
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
            name: secret.into(),
            value_ciphertext: encrypt_peer_secret(value, &KEY).unwrap(),
            created_by: member.id,
        })
        .await
        .unwrap();
    ws.id
}

async fn trust(store: &dyn Store, ws: WorkspaceId, host: &str) {
    store
        .allow_secret_egress_host(NewSecretEgressHost {
            workspace_id: ws,
            host: host.into(),
        })
        .await
        .unwrap();
}

const HOOK: &str = "https://hooks.example.com/x";
const BODY: &str = r#"{"auth":"Bearer secret://api-key","other":"secret://missing"}"#;

#[tokio::test]
async fn broker_substitutes_only_for_hosts_the_workspace_trusts() {
    let (state, store) = broker_state(None).await;
    let ws = workspace_with_secret(store.as_ref(), "w", "api-key", "TOPSECRET").await;

    // Nothing listed: nothing substituted.
    let untouched = secret_broker::substitute_for_egress(&state, ws, HOOK, BODY).await;
    assert_eq!(untouched, BODY, "an unlisted host gets the literal refs");

    trust(store.as_ref(), ws, "hooks.example.com").await;
    let subbed = secret_broker::substitute_for_egress(&state, ws, HOOK, BODY).await;
    assert!(subbed.contains("Bearer TOPSECRET"), "{subbed}");
    assert!(
        subbed.contains("secret://missing"),
        "unknown ref left literal"
    );
    assert!(!subbed.contains("secret://api-key"));

    for elsewhere in [
        "https://evil.example.com/x",
        "https://hooks.example.com.evil.com/x",
    ] {
        let untouched = secret_broker::substitute_for_egress(&state, ws, elsewhere, BODY).await;
        assert_eq!(untouched, BODY, "{elsewhere} got a value");
    }

    assert!(store
        .revoke_secret_egress_host(ws, "hooks.example.com")
        .await
        .unwrap());
    let untouched = secret_broker::substitute_for_egress(&state, ws, HOOK, BODY).await;
    assert_eq!(untouched, BODY, "a revoked host gets the literal refs");
}

#[tokio::test]
async fn one_workspaces_secret_never_reaches_another_workspaces_egress() {
    let (state, store) = broker_state(None).await;
    let a = workspace_with_secret(store.as_ref(), "a", "api-key", "VALUE-OF-A").await;
    let b = workspace_with_secret(store.as_ref(), "b", "b-only", "VALUE-OF-B").await;
    trust(store.as_ref(), a, "hooks.example.com").await;

    // A trusting the host does not let B's egress to it substitute anything.
    let from_b = secret_broker::substitute_for_egress(&state, b, HOOK, BODY).await;
    assert_eq!(from_b, BODY);

    // B trusting the same host resolves from B's secrets only: A's `api-key`
    // stays a literal ref in B's payload.
    trust(store.as_ref(), b, "hooks.example.com").await;
    let body = r#"{"a":"secret://api-key","b":"secret://b-only"}"#;
    let from_b = secret_broker::substitute_for_egress(&state, b, HOOK, body).await;
    assert!(!from_b.contains("VALUE-OF-A"), "{from_b}");
    assert!(from_b.contains("secret://api-key"));
    assert!(from_b.contains("VALUE-OF-B"));

    let from_a = secret_broker::substitute_for_egress(&state, a, HOOK, body).await;
    assert!(from_a.contains("VALUE-OF-A") && !from_a.contains("VALUE-OF-B"));
}

#[tokio::test]
async fn the_instance_ceiling_bounds_what_a_workspace_can_trust() {
    let (state, store) = broker_state(Some(vec!["other.example.com".into()])).await;
    let ws = workspace_with_secret(store.as_ref(), "w", "api-key", "TOPSECRET").await;
    // Listed in the store directly: the ceiling holds at send time too, for a
    // host listed before the operator narrowed it.
    trust(store.as_ref(), ws, "hooks.example.com").await;
    let untouched = secret_broker::substitute_for_egress(&state, ws, HOOK, BODY).await;
    assert_eq!(
        untouched, BODY,
        "a host outside the ceiling gets the literal refs"
    );

    let (off, store) = broker_state(Some(vec![])).await;
    let ws = workspace_with_secret(store.as_ref(), "w", "api-key", "TOPSECRET").await;
    trust(store.as_ref(), ws, "hooks.example.com").await;
    let untouched = secret_broker::substitute_for_egress(&off, ws, HOOK, BODY).await;
    assert_eq!(untouched, BODY, "an empty ceiling admits no host");
}

#[tokio::test]
async fn a_value_with_quotes_and_newlines_leaves_the_payload_valid_json() {
    let (state, store) = broker_state(None).await;
    let pem = "-----BEGIN KEY-----\nab\"c\\d\n-----END KEY-----";
    let ws = workspace_with_secret(store.as_ref(), "w", "pem", pem).await;
    trust(store.as_ref(), ws, "hooks.example.com").await;
    let body = r#"{"key":"secret://pem","n":1}"#;
    let subbed = secret_broker::substitute_for_egress(&state, ws, HOOK, body).await;
    let parsed: serde_json::Value = serde_json::from_str(&subbed).expect("still JSON");
    assert_eq!(parsed["key"], pem);
    assert_eq!(parsed["n"], 1);
}
