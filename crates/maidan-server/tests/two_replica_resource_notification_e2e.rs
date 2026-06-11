//! Cross-replica MCP resource notifications (Cluster 102.0.4).
//!
//! Two `McpServer`s sharing one Postgres database stand in for two server
//! replicas behind a load balancer, each with its own
//! `PostgresResourceNotifier` and listener. A client subscribed on replica A
//! must receive `notifications/resources/updated` when the corresponding
//! mutation is handled on replica B — behavior that per-process in-memory
//! notifications could not provide before this cluster.

use std::sync::Arc;
use std::time::Duration;

use maidan_auth::AuthContext;
use maidan_bus::{PostgresResourceNotifier, ResourceNotifier};
use maidan_mcp::{JsonRpcRequest, McpServer};
use maidan_search::{EmbeddingProvider, HashV1Provider, PostgresSearch, Search};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

fn subscribe_request(uri: &str) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "resources/subscribe".into(),
        params: json!({ "uri": uri }),
    }
}

#[tokio::test]
async fn resource_update_on_one_replica_reaches_subscriber_on_another() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping two-replica notify e2e: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();

    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let artifacts: Arc<dyn maidan_artifacts::ArtifactStore> =
        Arc::new(maidan_artifacts::LocalFsStore::new(
            std::env::temp_dir().join(format!("maidan-102-{}", uuid::Uuid::new_v4())),
        ));
    let search: Arc<dyn Search> = Arc::new(PostgresSearch::new(pool.clone()));
    let provider: Arc<dyn EmbeddingProvider> = Arc::new(HashV1Provider);

    // A real thread (shared DB) gives a valid resource URI to subscribe to.
    let ws = store
        .create_workspace(NewWorkspace {
            name: "notify-ws".into(),
        })
        .await
        .unwrap();
    let _member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let uri = format!("maidan://threads/{}", thread.id.0);

    // Two replicas, each with its own Postgres resource notifier + listener.
    let notifier_a: Arc<dyn ResourceNotifier> = Arc::new(
        PostgresResourceNotifier::connect(pool.clone())
            .await
            .unwrap(),
    );
    let notifier_b: Arc<dyn ResourceNotifier> = Arc::new(
        PostgresResourceNotifier::connect(pool.clone())
            .await
            .unwrap(),
    );
    let replica_a = Arc::new(
        McpServer::new(
            store.clone(),
            artifacts.clone(),
            search.clone(),
            provider.clone(),
        )
        .with_resource_notifier(notifier_a),
    );
    let replica_b = Arc::new(
        McpServer::new(
            store.clone(),
            artifacts.clone(),
            search.clone(),
            provider.clone(),
        )
        .with_resource_notifier(notifier_b),
    );
    replica_a.spawn_resource_notify_listener();
    replica_b.spawn_resource_notify_listener();
    // Let both LISTEN tasks attach before publishing.
    tokio::time::sleep(Duration::from_millis(400)).await;

    // The subscriber lives on replica A.
    let auth = AuthContext::bypass();
    let sub = replica_a.handle(subscribe_request(&uri), &auth).await;
    assert!(sub.error.is_none(), "subscribe failed: {sub:?}");
    let mut sse_a = replica_a.subscribe_notifications();

    // A mutation handled on replica B fans the URI out cross-replica.
    replica_b.publish_resource_uris(vec![uri.clone()]).await;

    // Replica A's listener delivers it to A's own SSE subscriber.
    let got = tokio::time::timeout(Duration::from_secs(5), sse_a.recv())
        .await
        .expect("timeout waiting for cross-replica resource notification")
        .expect("notification channel closed");
    assert_eq!(got.method, "notifications/resources/updated");
    assert_eq!(got.params["uri"], uri);

    // A URI the subscriber did NOT subscribe to must not be delivered.
    replica_b
        .publish_resource_uris(vec![
            "maidan://threads/00000000-0000-0000-0000-000000000000".into(),
        ])
        .await;
    let unexpected = tokio::time::timeout(Duration::from_millis(500), sse_a.recv()).await;
    assert!(
        unexpected.is_err(),
        "unsubscribed URI should not be delivered, got {unexpected:?}"
    );
}
