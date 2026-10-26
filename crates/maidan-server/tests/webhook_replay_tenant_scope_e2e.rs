//! A workspace can replay or revoke only its own webhooks.
//!
//! On SQLite the replay used to un-quarantine the delivery row before it
//! checked the workspace, so a caller with write access to one workspace could
//! re-send another workspace's dead delivery by its sequential id and charge
//! that workspace's allowance. Revoking a subscription had the same shape on
//! both backends. Each scenario runs on SQLite and on Postgres.

mod common;

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations, AutomationDeliveryFilter};
use maidan_types::{
    MemberKind, NewApiToken, NewMember, NewWebhookSubscription, NewWorkspace,
    WebhookSubscriptionId, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

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

/// A workspace with one writer and a bearer token for it.
async fn tenant(store: &dyn Store, name: &str) -> (WorkspaceId, String) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: format!("{name}-writer"),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws, secret.as_str().to_string())
}

async fn subscription(store: &dyn Store, ws: WorkspaceId) -> WebhookSubscriptionId {
    store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws,
            url: "http://127.0.0.1:9/hook".into(),
            label: None,
            event_kinds: vec!["message.posted".into()],
            secret_ciphertext: "cipher".into(),
        })
        .await
        .unwrap()
        .id
}

/// A quarantined delivery for `sub` that has already failed twice.
async fn dead_delivery(store: &dyn Store, sub: WebhookSubscriptionId) -> i64 {
    let id = store
        .enqueue_webhook_delivery(sub, 1, r#"{"kind":"message.posted"}"#)
        .await
        .unwrap();
    for _ in 0..2 {
        store
            .record_webhook_delivery_attempt(id, "refused", chrono::Utc::now())
            .await
            .unwrap();
    }
    store.quarantine_webhook_delivery(id).await.unwrap();
    id
}

impl Harness {
    async fn replay(&self, token: &str, ws: WorkspaceId, id: i64) -> reqwest::Response {
        self.client
            .post(format!(
                "{}/workspaces/{}/deliveries/{id}/replay",
                self.base, ws.0
            ))
            .query(&[("kind", "webhook")])
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }

    async fn revoke(
        &self,
        token: &str,
        ws: WorkspaceId,
        id: WebhookSubscriptionId,
    ) -> reqwest::Response {
        self.client
            .delete(format!(
                "{}/workspaces/{}/webhooks/{}",
                self.base, ws.0, id.0
            ))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }
}

async fn another_workspace_cannot_replay_a_dead_delivery_and_its_owner_still_can(h: &Harness) {
    let store = h.store.as_ref();
    let (a, a_token) = tenant(store, "alpha").await;
    let (b, b_token) = tenant(store, "bravo").await;
    let a_delivery = dead_delivery(store, subscription(store, a).await).await;
    let _b_delivery = dead_delivery(store, subscription(store, b).await).await;

    let resp = h.replay(&b_token, b, a_delivery).await;
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);

    let row = store.get_webhook_delivery(a_delivery, a).await.unwrap();
    assert!(
        row.quarantined_at.is_some(),
        "A's delivery stays quarantined"
    );
    assert_eq!(row.attempts, 2, "A's attempts are unchanged");
    assert_eq!(row.last_error.as_deref(), Some("refused"));
    let pending = store
        .list_webhook_deliveries(a, AutomationDeliveryFilter::Pending, 10)
        .await
        .unwrap();
    assert!(pending.iter().all(|d| d.id != a_delivery));

    let resp = h.replay(&a_token, a, a_delivery).await;
    assert!(
        resp.status().is_success(),
        "owner replay: {}",
        resp.status()
    );
    let row = store.get_webhook_delivery(a_delivery, a).await.unwrap();
    assert!(row.quarantined_at.is_none());
    assert_eq!(row.attempts, 0);
}

async fn another_workspace_cannot_revoke_a_subscription_and_its_owner_still_can(h: &Harness) {
    let store = h.store.as_ref();
    let (a, a_token) = tenant(store, "alpha").await;
    let (b, b_token) = tenant(store, "bravo").await;
    let a_sub = subscription(store, a).await;

    let resp = h.revoke(&b_token, b, a_sub).await;
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let live = store.list_webhook_subscriptions(a).await.unwrap();
    assert!(live.iter().any(|s| s.id == a_sub && s.enabled));

    let resp = h.revoke(&a_token, a, a_sub).await;
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);
    let live = store.list_webhook_subscriptions(a).await.unwrap();
    assert!(live.iter().all(|s| s.id != a_sub));
}

#[tokio::test]
async fn another_workspace_cannot_replay_a_dead_delivery_and_its_owner_still_can_on_sqlite() {
    another_workspace_cannot_replay_a_dead_delivery_and_its_owner_still_can(
        &sqlite_harness().await,
    )
    .await;
}

#[tokio::test]
async fn another_workspace_cannot_replay_a_dead_delivery_and_its_owner_still_can_on_postgres() {
    let Some((_pg, h)) = postgres_harness().await else {
        return;
    };
    another_workspace_cannot_replay_a_dead_delivery_and_its_owner_still_can(&h).await;
}

#[tokio::test]
async fn another_workspace_cannot_revoke_a_subscription_and_its_owner_still_can_on_sqlite() {
    another_workspace_cannot_revoke_a_subscription_and_its_owner_still_can(&sqlite_harness().await)
        .await;
}

#[tokio::test]
async fn another_workspace_cannot_revoke_a_subscription_and_its_owner_still_can_on_postgres() {
    let Some((_pg, h)) = postgres_harness().await else {
        return;
    };
    another_workspace_cannot_revoke_a_subscription_and_its_owner_still_can(&h).await;
}
