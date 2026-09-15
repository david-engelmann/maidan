//! Cluster 392: hash chain on append; tamper is detected. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    content_hash, genesis_hash, ChainBreakReason, Event, MemberKind, NewChannel, NewMember,
    NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn seed(store: &dyn Store) -> (maidan_types::WorkspaceId, maidan_types::Member) {
    // Plain `create_*` does not append to the log. The chain is only
    // written by `append` / `*_with_event`.
    let (ws, stored_ws) = store
        .create_workspace_with_event(NewWorkspace {
            name: "chain-ws".into(),
        })
        .await
        .expect("ws");
    assert_eq!(stored_ws.prev_hash, genesis_hash());
    assert_eq!(stored_ws.lsn, stored_ws.id);
    let (member, stored_member) = store
        .create_member_with_event(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    assert_ne!(stored_member.prev_hash, genesis_hash());
    let _ch = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    (ws.id, member)
}

async fn run_suite(store: &dyn Store) {
    let report = store
        .verify_event_chain(maidan_types::WorkspaceId(uuid::Uuid::nil()))
        .await
        .expect("empty unknown ws");
    assert!(report.ok);
    assert_eq!(report.checked, 0);

    let (ws, member) = seed(store).await;
    let report = store.verify_event_chain(ws).await.expect("after seed");
    assert!(report.ok, "{report:?}");
    assert!(report.checked >= 2, "workspace + member (+ channel) events");
    assert!(report.from_genesis);
    let head = report.head.expect("head");
    assert_eq!(head.lsn, head.id);
    assert!(head.prev_hash.starts_with("sha256:"));
    assert!(head.content_hash.starts_with("sha256:"));

    let extra = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member: member.clone(),
        })
        .await
        .expect("append");
    assert_eq!(extra.lsn, extra.id);
    assert_ne!(extra.prev_hash, genesis_hash());
    assert_eq!(
        extra.content_hash,
        content_hash(&extra.payload).expect("hash")
    );
    let report = store.verify_event_chain(ws).await.expect("after append");
    assert!(report.ok, "{report:?}");
    assert_eq!(report.head.as_ref().map(|h| h.id), Some(extra.id));

    let (ws_b, stored_b) = store
        .create_workspace_with_event(NewWorkspace {
            name: "other".into(),
        })
        .await
        .expect("ws b");
    let report_b = store.verify_event_chain(ws_b.id).await.expect("b");
    assert!(report_b.ok);
    assert!(report_b.from_genesis);
    assert_eq!(report_b.checked, 1);
    assert_eq!(stored_b.prev_hash, genesis_hash());
    // Other-tenant appends must not fork this workspace's chain.
    let still = store.verify_event_chain(ws).await.expect("a isolated");
    assert!(still.ok);
    assert_eq!(still.head.as_ref().map(|h| h.id), Some(extra.id));
}

#[tokio::test]
async fn append_then_verify_ok_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn tamper_is_detected_sqlite() {
    let store = sqlite().await;
    let (ws, member) = seed(&store).await;
    let stored = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member,
        })
        .await
        .expect("append");
    assert!(store.verify_event_chain(ws).await.expect("pre").ok);

    let mut payload = stored.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(stored.id)
        .execute(store.pool())
        .await
        .expect("tamper");

    let report = store.verify_event_chain(ws).await.expect("verify");
    assert!(!report.ok);
    assert_eq!(report.break_at, Some(stored.id));
    assert_eq!(report.reason, Some(ChainBreakReason::ContentHashMismatch));
}

#[tokio::test]
async fn prev_hash_break_is_detected_sqlite() {
    let store = sqlite().await;
    let (ws, member) = seed(&store).await;
    let stored = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member,
        })
        .await
        .expect("append");
    sqlx::query("UPDATE maidan_events SET prev_hash = ? WHERE id = ?")
        .bind(genesis_hash())
        .bind(stored.id)
        .execute(store.pool())
        .await
        .expect("tamper prev");
    let report = store.verify_event_chain(ws).await.expect("verify");
    assert!(!report.ok);
    assert_eq!(report.break_at, Some(stored.id));
    assert_eq!(report.reason, Some(ChainBreakReason::PrevHashMismatch));
}

#[tokio::test]
async fn append_then_verify_ok_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}

#[tokio::test]
async fn tamper_is_detected_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    let (ws, member) = seed(&store).await;
    let stored = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member,
        })
        .await
        .expect("append");
    assert!(store.verify_event_chain(ws).await.expect("pre").ok);

    let mut payload = stored.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = $1 WHERE id = $2")
        .bind(&payload)
        .bind(stored.id)
        .execute(store.pool())
        .await
        .expect("tamper");

    let report = store.verify_event_chain(ws).await.expect("verify");
    assert!(!report.ok);
    assert_eq!(report.break_at, Some(stored.id));
    assert_eq!(report.reason, Some(ChainBreakReason::ContentHashMismatch));
}
