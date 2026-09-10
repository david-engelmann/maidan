//! SCIM 2.0 provisioning over HTTP (Cluster 366, SCIM-as-OIDC-P3): the full
//! lifecycle — create, read, list-with-filter, duplicate 409, PATCH deactivate
//! (revokes the member's tokens), delete — plus the token:admin gate. Auth-enabled
//! with a minted token:admin bearer.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    base: String,
    client: reqwest::Client,
    admin_auth: String,
    store: Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    _server: tokio::task::JoinHandle<()>,
}

async fn setup() -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let admin = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "idp".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: admin.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("idp".into()),
            capabilities: vec!["token:admin".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    Harness {
        base: format!("http://{addr}"),
        client,
        admin_auth: format!("Bearer {}", secret.as_str()),
        store,
        ws: ws.id,
        _server: server,
    }
}

#[tokio::test]
async fn scim_user_lifecycle_over_http() {
    let h = setup().await;
    let users = format!("{}/scim/v2/Users", h.base);

    // ServiceProviderConfig advertises patch + filter support.
    let spc: serde_json::Value = h
        .client
        .get(format!("{}/scim/v2/ServiceProviderConfig", h.base))
        .header("Authorization", &h.admin_auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(spc["patch"]["supported"], true);
    assert_eq!(spc["filter"]["supported"], true);

    // Create.
    let created = h
        .client
        .post(&users)
        .header("Authorization", &h.admin_auth)
        .json(&serde_json::json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "userName": "alice",
            "displayName": "Alice A",
            "externalId": "okta-1",
            "active": true,
            "emails": [{ "value": "alice@example.com", "primary": true }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body: serde_json::Value = created.json().await.unwrap();
    assert_eq!(body["userName"], "alice");
    assert_eq!(body["externalId"], "okta-1");
    assert_eq!(body["active"], true);
    let user_id = body["id"].as_str().unwrap().to_string();
    let member_id = MemberId(user_id.parse().unwrap());

    // Duplicate userName → 409.
    assert_eq!(
        h.client
            .post(&users)
            .header("Authorization", &h.admin_auth)
            .json(&serde_json::json!({ "userName": "alice" }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );

    // Get by id.
    assert_eq!(
        h.client
            .get(format!("{users}/{user_id}"))
            .header("Authorization", &h.admin_auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    // List with userName filter → exactly one.
    let list: serde_json::Value = h
        .client
        .get(format!("{users}?filter=userName%20eq%20%22alice%22"))
        .header("Authorization", &h.admin_auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["totalResults"], 1);
    assert_eq!(list["Resources"][0]["userName"], "alice");

    // Give the provisioned member a token, then PATCH active=false → it's revoked.
    let member_secret = TokenSecret::generate();
    let token = h
        .store
        .create_api_token(NewApiToken {
            workspace_id: h.ws,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(member_secret.as_str()),
            label: Some("alice".into()),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let patched: serde_json::Value = h
        .client
        .patch(format!("{users}/{user_id}"))
        .header("Authorization", &h.admin_auth)
        .json(&serde_json::json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{ "op": "replace", "path": "active", "value": false }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(patched["active"], false);
    let toks = h
        .store
        .list_api_tokens_for_member(h.ws, member_id)
        .await
        .unwrap();
    assert!(
        toks.iter()
            .find(|t| t.id == token.id)
            .unwrap()
            .revoked_at
            .is_some(),
        "deactivation revokes the member's token"
    );

    // Delete → 204, then Get → 404.
    assert_eq!(
        h.client
            .delete(format!("{users}/{user_id}"))
            .header("Authorization", &h.admin_auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.client
            .get(format!("{users}/{user_id}"))
            .header("Authorization", &h.admin_auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn scim_requires_token_admin() {
    let h = setup().await;
    // A workspace:read-only bearer is forbidden from SCIM.
    let member = h
        .store
        .create_member(NewMember {
            workspace_id: h.ws,
            handle: "weak".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    h.store
        .create_api_token(NewApiToken {
            workspace_id: h.ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let resp = h
        .client
        .get(format!("{}/scim/v2/Users", h.base))
        .header("Authorization", format!("Bearer {}", secret.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
