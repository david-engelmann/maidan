//! Postgres message edit (Cluster 29).

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::{
    EditMessage, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn postgres_edit_message_sets_edited_at() {
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
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "pg-edit".into(),
        })
        .await
        .unwrap();
    let member = store
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
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "before".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let updated = store
        .edit_message(
            msg.id,
            member.id,
            EditMessage {
                body: "after".into(),
                metadata: serde_json::json!({"v": 1}),
                content: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.body, "after");
    assert!(updated.edited_at.is_some());
    let history = store.list_message_edits(msg.id, 10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].body_before, "before");
    assert_eq!(history[0].body_after, "after");
}

/// Cluster 173: typed content round-trips through the Postgres JSONB column.
#[tokio::test]
async fn postgres_message_content_round_trips_via_jsonb() {
    use maidan_types::ContentBlock;
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
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "pg-content".into(),
        })
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
    let blocks = vec![
        ContentBlock::Text { text: "hi".into() },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "shell".into(),
            input: serde_json::json!({"cmd": "ls"}),
        },
    ];
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "hi".into(),
            metadata: serde_json::json!({}),
            content: Some(blocks.clone()),
        })
        .await
        .unwrap();
    assert_eq!(msg.content.as_deref(), Some(blocks.as_slice()));

    // Re-read via get + list to prove the JSONB column deserializes.
    let got = store.get_message(msg.id).await.unwrap();
    assert_eq!(got.content, Some(blocks.clone()));
    let listed = store.list_messages(thread.id, 10).await.unwrap();
    assert_eq!(listed[0].content, Some(blocks));

    // Tombstone nulls content.
    store.tombstone_message(msg.id).await.unwrap();
    // A tombstoned message is excluded from list; fetch directly.
    let after = store.get_message(msg.id).await.unwrap();
    assert!(after.content.is_none(), "tombstone clears content");
}
