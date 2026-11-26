//! A webhook list says which token each webhook acts as.
//!
//! A webhook is revoked at its next event once the token that registered it
//! ends, so `GET /workspaces/{wid}/webhooks` names that token (its id and
//! label, never a secret) and says whether it is live, expiring or ended. Only
//! a caller who may list the workspace's tokens (`token:admin`) sees it;
//! another caller gets the webhook without those fields, and a workspace never
//! describes another workspace's token. Each scenario runs on SQLite and on
//! Postgres.

mod common;

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use chrono::{DateTime, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, test_support, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApiTokenId, MemberKind, NewApiToken, NewMember, NewWebhookSubscription, WebhookSubscriptionId,
    WorkspaceId,
};
use serde_json::Value;

struct Harness {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn spawn(store: Arc<dyn Store>, search: Arc<dyn maidan_search::Search>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        server,
        _dir: dir,
    }
}

async fn sqlite_harness() -> Harness {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    spawn(store, Arc::new(maidan_search::SqliteSearch::new(pool))).await
}

async fn postgres_harness() -> Option<(
    testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    Harness,
)> {
    let (container, pool) = common::postgres_pool().await?;
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let harness = spawn(store, Arc::new(maidan_search::PostgresSearch::new(pool))).await;
    Some((container, harness))
}

/// A token of a new member of `ws`, with its id and bearer secret.
async fn mint(
    store: &dyn Store,
    ws: WorkspaceId,
    handle: &str,
    label: &str,
    capabilities: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
) -> (ApiTokenId, String) {
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some(label.into()),
            capabilities,
            expires_at,
        })
        .await
        .unwrap();
    (token.id, secret.as_str().to_string())
}

async fn webhook(
    store: &dyn Store,
    ws: WorkspaceId,
    label: &str,
    created_by_token: Option<ApiTokenId>,
) -> WebhookSubscriptionId {
    store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws,
            url: "http://127.0.0.1:9/hook".into(),
            label: Some(label.into()),
            event_kinds: vec!["message.posted".into()],
            secret_ciphertext: "cipher".into(),
            created_by_token,
        })
        .await
        .unwrap()
        .id
}

impl Harness {
    async fn list(&self, token: &str, ws: WorkspaceId) -> (reqwest::StatusCode, Value) {
        let resp = self
            .client
            .get(format!("{}/workspaces/{}/webhooks", self.base, ws.0))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }
}

/// The listed webhook labelled `label`.
fn labelled<'a>(list: &'a Value, label: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|w| w["label"] == label)
        .unwrap_or_else(|| panic!("no webhook labelled {label} in {list}"))
}

async fn a_token_admin_sees_each_webhooks_token_and_whether_it_is_live(h: &Harness) {
    let store = h.store.as_ref();
    let tenant = test_support::tenant(store, "alpha").await;
    let all = capability::all();
    let soon = Utc::now() + chrono::Duration::days(3);
    let (live, _) = mint(
        store,
        tenant.workspace_id,
        "live",
        "ops-live",
        all.clone(),
        None,
    )
    .await;
    let (expiring, _) = mint(
        store,
        tenant.workspace_id,
        "expiring",
        "ops-expiring",
        all.clone(),
        Some(soon),
    )
    .await;
    let (revoked, _) = mint(
        store,
        tenant.workspace_id,
        "revoked",
        "ops-revoked",
        all.clone(),
        None,
    )
    .await;
    let (expired, _) = mint(
        store,
        tenant.workspace_id,
        "expired",
        "ops-expired",
        all.clone(),
        Some(Utc::now() - chrono::Duration::days(1)),
    )
    .await;
    // A token that never held what registering a webhook needs.
    let (reader, _) = mint(
        store,
        tenant.workspace_id,
        "reader",
        "ops-reader",
        vec![capability::WORKSPACE_READ.into()],
        None,
    )
    .await;
    store.revoke_api_token(revoked).await.unwrap();
    let ws = tenant.workspace_id;
    webhook(store, ws, "w-live", Some(live)).await;
    webhook(store, ws, "w-expiring", Some(expiring)).await;
    webhook(store, ws, "w-revoked", Some(revoked)).await;
    webhook(store, ws, "w-expired", Some(expired)).await;
    webhook(store, ws, "w-reader", Some(reader)).await;
    webhook(store, ws, "w-none", None).await;

    let (status, list) = h.list(&tenant.token, ws).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{list}");
    assert_eq!(list.as_array().unwrap().len(), 6);

    let acts = &labelled(&list, "w-live")["acts_as"];
    assert_eq!(acts["token_id"], live.0.to_string());
    assert_eq!(acts["label"], "ops-live");
    assert_eq!(acts["status"], "live");
    assert!(acts["expires_at"].is_null());

    let acts = &labelled(&list, "w-expiring")["acts_as"];
    assert_eq!(acts["token_id"], expiring.0.to_string());
    assert_eq!(acts["label"], "ops-expiring");
    assert_eq!(acts["status"], "expiring");
    let at: DateTime<Utc> = acts["expires_at"].as_str().unwrap().parse().unwrap();
    assert!((at - soon).num_seconds().abs() <= 1, "expires at {at}");

    let acts = &labelled(&list, "w-revoked")["acts_as"];
    assert_eq!(acts["token_id"], revoked.0.to_string());
    assert_eq!(acts["label"], "ops-revoked");
    assert_eq!(acts["status"], "ended");

    let acts = &labelled(&list, "w-expired")["acts_as"];
    assert_eq!(acts["label"], "ops-expired");
    assert_eq!(acts["status"], "ended");
    assert!(acts["expires_at"].is_string());

    let acts = &labelled(&list, "w-reader")["acts_as"];
    assert_eq!(acts["label"], "ops-reader");
    assert_eq!(
        acts["status"], "ended",
        "a token without workspace:admin cannot register the webhook again"
    );

    let acts = &labelled(&list, "w-none")["acts_as"];
    assert!(acts["token_id"].is_null());
    assert_eq!(acts["status"], "ended");

    // Never a secret: the webhook's signing secret or any token's hash.
    let text = list.to_string();
    assert!(!text.contains("cipher"), "{text}");
    assert!(!text.contains("token_hash"), "{text}");
    assert!(!text.contains("secret"), "{text}");
}

async fn a_caller_who_may_not_list_tokens_gets_the_webhook_without_the_token(h: &Harness) {
    let store = h.store.as_ref();
    let tenant = test_support::tenant(store, "alpha").await;
    let ws = tenant.workspace_id;
    let (creator, _) = mint(
        store,
        ws,
        "creator",
        "creator-label",
        capability::all(),
        None,
    )
    .await;
    webhook(store, ws, "w", Some(creator)).await;

    // Everything an administrator holds short of listing tokens.
    let without: Vec<String> = capability::all()
        .into_iter()
        .filter(|c| c != capability::TOKEN_ADMIN)
        .collect();
    let (_, no_tokens) = mint(store, ws, "no-tokens", "no-tokens", without, None).await;
    let (_, reader) = mint(
        store,
        ws,
        "reader",
        "reader",
        vec![capability::WORKSPACE_READ.into()],
        None,
    )
    .await;
    let (_, token_admin) = mint(
        store,
        ws,
        "token-admin",
        "token-admin",
        vec![
            capability::WORKSPACE_READ.into(),
            capability::TOKEN_ADMIN.into(),
        ],
        None,
    )
    .await;

    for (who, bearer) in [("no token:admin", &no_tokens), ("reader", &reader)] {
        let (status, list) = h.list(bearer, ws).await;
        assert_eq!(status, reqwest::StatusCode::OK, "{who}: {list}");
        let hook = labelled(&list, "w");
        assert!(
            hook.get("acts_as").is_none(),
            "{who} must not see the token: {hook}"
        );
        let text = list.to_string();
        assert!(!text.contains("creator-label"), "{who}: {text}");
        assert!(!text.contains(&creator.0.to_string()), "{who}: {text}");
    }

    let (status, list) = h.list(&token_admin, ws).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{list}");
    assert_eq!(labelled(&list, "w")["acts_as"]["label"], "creator-label");
    assert_eq!(labelled(&list, "w")["acts_as"]["status"], "live");
}

async fn a_workspace_never_describes_another_workspaces_token(h: &Harness) {
    let store = h.store.as_ref();
    let a = test_support::tenant(store, "alpha").await;
    let b = test_support::tenant(store, "bravo").await;
    let (a_token, _) = mint(
        store,
        a.workspace_id,
        "a-registrar",
        "alpha-secret-label",
        capability::all(),
        None,
    )
    .await;
    let (b_token, _) = mint(
        store,
        b.workspace_id,
        "b-registrar",
        "bravo-label",
        capability::all(),
        None,
    )
    .await;
    webhook(store, a.workspace_id, "a-hook", Some(a_token)).await;
    webhook(store, b.workspace_id, "b-hook", Some(b_token)).await;
    // Bravo's webhook naming alpha's live token however it got there.
    webhook(store, b.workspace_id, "b-foreign", Some(a_token)).await;

    let (status, list) = h.list(&b.token, b.workspace_id).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{list}");
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(labelled(&list, "b-hook")["acts_as"]["label"], "bravo-label");
    assert_eq!(labelled(&list, "b-hook")["acts_as"]["status"], "live");
    let foreign = &labelled(&list, "b-foreign")["acts_as"];
    assert_eq!(foreign["status"], "ended");
    assert!(foreign["label"].is_null(), "{foreign}");
    assert!(
        !list.to_string().contains("alpha-secret-label"),
        "bravo's list must not carry alpha's token label"
    );

    // Alpha's list carries alpha's hook only, and bravo's token is refused on it.
    let (_, alpha_list) = h.list(&a.token, a.workspace_id).await;
    assert_eq!(alpha_list.as_array().unwrap().len(), 1);
    assert_eq!(
        labelled(&alpha_list, "a-hook")["acts_as"]["label"],
        "alpha-secret-label"
    );
    let (status, body) = h.list(&b.token, a.workspace_id).await;
    assert!(
        status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND,
        "{status}"
    );
    assert!(!body.to_string().contains("alpha-secret-label"), "{body}");
}

#[tokio::test]
async fn a_token_admin_sees_each_webhooks_token_and_whether_it_is_live_on_sqlite() {
    a_token_admin_sees_each_webhooks_token_and_whether_it_is_live(&sqlite_harness().await).await;
}

#[tokio::test]
async fn a_token_admin_sees_each_webhooks_token_and_whether_it_is_live_on_postgres() {
    let Some((_pg, h)) = postgres_harness().await else {
        return;
    };
    a_token_admin_sees_each_webhooks_token_and_whether_it_is_live(&h).await;
}

#[tokio::test]
async fn a_caller_who_may_not_list_tokens_gets_the_webhook_without_the_token_on_sqlite() {
    a_caller_who_may_not_list_tokens_gets_the_webhook_without_the_token(&sqlite_harness().await)
        .await;
}

#[tokio::test]
async fn a_caller_who_may_not_list_tokens_gets_the_webhook_without_the_token_on_postgres() {
    let Some((_pg, h)) = postgres_harness().await else {
        return;
    };
    a_caller_who_may_not_list_tokens_gets_the_webhook_without_the_token(&h).await;
}

#[tokio::test]
async fn a_workspace_never_describes_another_workspaces_token_on_sqlite() {
    a_workspace_never_describes_another_workspaces_token(&sqlite_harness().await).await;
}

#[tokio::test]
async fn a_workspace_never_describes_another_workspaces_token_on_postgres() {
    let Some((_pg, h)) = postgres_harness().await else {
        return;
    };
    a_workspace_never_describes_another_workspaces_token(&h).await;
}
