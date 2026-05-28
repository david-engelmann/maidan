//! Postgres message edit (Cluster 29).

use std::time::Duration;

use maidan_store::{run_postgres_migrations, PostgresStore, Store};
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
        })
        .await
        .unwrap();

    let updated = store
        .edit_message(
            msg.id,
            EditMessage {
                body: "after".into(),
                metadata: serde_json::json!({"v": 1}),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.body, "after");
    assert!(updated.edited_at.is_some());
}
