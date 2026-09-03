//! Workspace import (Cluster 269): a `WorkspaceImport` graph inserts with its ids,
//! state, and timestamps preserved, and reads back faithfully. Both backends.

use chrono::Utc;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;

fn sample() -> WorkspaceImport {
    let now = Utc::now();
    let ws = WorkspaceId(uuid::Uuid::new_v4());
    let m1 = MemberId(uuid::Uuid::new_v4());
    let m2 = MemberId(uuid::Uuid::new_v4());
    let ch = ChannelId(uuid::Uuid::new_v4());
    let th = ThreadId(uuid::Uuid::new_v4());
    let msg1 = MessageId(uuid::Uuid::new_v4());
    let msg2 = MessageId(uuid::Uuid::new_v4());
    let lease = ClaimLeaseId(uuid::Uuid::new_v4());
    WorkspaceImport {
        workspace: Workspace {
            id: ws,
            name: "imported".into(),
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        },
        members: vec![
            Member {
                id: m1,
                workspace_id: ws,
                handle: "alice".into(),
                display_name: Some("Alice".into()),
                kind: MemberKind::Human,
                created_at: now,
                updated_at: now,
                tombstoned_at: None,
            },
            Member {
                id: m2,
                workspace_id: ws,
                handle: "bot".into(),
                display_name: None,
                kind: MemberKind::Agent,
                created_at: now,
                updated_at: now,
                tombstoned_at: None,
            },
        ],
        channels: vec![Channel {
            id: ch,
            workspace_id: ws,
            name: "general".into(),
            topic: Some("hi".into()),
            private: true,
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        }],
        channel_members: vec![
            ChannelMember {
                channel_id: ch,
                member_id: m1,
                role: ChannelMemberRole::Admin,
                created_at: now,
            },
            ChannelMember {
                channel_id: ch,
                member_id: m2,
                role: ChannelMemberRole::Member,
                created_at: now,
            },
        ],
        threads: vec![Thread {
            id: th,
            channel_id: ch,
            parent_thread_id: None,
            title: Some("t".into()),
            state: ThreadState::Closed,
            assignee_id: Some(m2),
            assignment_expires_at: None,
            claim_lease_id: Some(lease),
            work_started_at: Some(now),
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        }],
        messages: vec![
            Message {
                id: msg1,
                thread_id: th,
                author_id: m1,
                body: "hello".into(),
                metadata: serde_json::json!({}),
                content: Some(vec![ContentBlock::Text {
                    text: "hello".into(),
                }]),
                posted_at: now,
                edited_at: None,
                tombstoned_at: None,
            },
            Message {
                id: msg2,
                thread_id: th,
                author_id: m2,
                body: "gone".into(),
                metadata: serde_json::json!({}),
                content: None,
                posted_at: now,
                edited_at: None,
                tombstoned_at: Some(now),
            },
        ],
        message_edits: vec![MessageEdit {
            id: 0,
            message_id: msg1,
            editor_id: m1,
            body_before: "hi".into(),
            body_after: "hello".into(),
            edited_at: now,
        }],
        pins: vec![Pin {
            thread_id: th,
            message_id: msg1,
            member_id: m1,
            created_at: now,
        }],
        references: vec![Reference {
            id: uuid::Uuid::new_v4(),
            src_kind: RefSide::Thread,
            src_id: th.0,
            dst_kind: RefSide::Message,
            dst_id: msg1.0,
            relation: "about".into(),
            created_at: now,
        }],
    }
}

async fn run_suite(store: &dyn Store) {
    let b = sample();
    store.import_workspace(&b).await.expect("import");

    let ws = b.workspace.id;
    let got = store.get_workspace(ws).await.expect("workspace");
    assert_eq!(got.name, "imported");

    let members = store.list_members(ws).await.expect("members");
    assert_eq!(members.len(), 2);

    let channels = store.list_channels(ws).await.expect("channels");
    assert_eq!(channels.len(), 1);
    assert!(channels[0].private, "private flag preserved");
    let cms = store
        .list_channel_members(channels[0].id)
        .await
        .expect("channel members");
    assert_eq!(cms.len(), 2);

    let threads = store.list_threads(channels[0].id).await.expect("threads");
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].state, ThreadState::Closed, "state preserved");
    assert_eq!(
        threads[0].assignee_id,
        Some(b.members[1].id),
        "assignee preserved"
    );
    assert_eq!(
        threads[0].claim_lease_id, b.threads[0].claim_lease_id,
        "claim lease id preserved through import"
    );
    assert_eq!(
        threads[0].work_started_at.is_some(),
        b.threads[0].work_started_at.is_some(),
        "working clock preserved through import"
    );

    let msgs = store
        .list_messages(threads[0].id, 100)
        .await
        .expect("messages");
    // Tombstoned messages are typically excluded from the live list; the live one
    // is present with its structured content.
    let live = msgs
        .iter()
        .find(|m| m.body == "hello")
        .expect("live message");
    assert_eq!(
        live.content.as_deref(),
        Some(
            &[ContentBlock::Text {
                text: "hello".into()
            }][..]
        ),
        "content preserved"
    );

    let pins = store
        .list_pins_for_thread(threads[0].id)
        .await
        .expect("pins");
    assert_eq!(pins.len(), 1);

    let edits = store
        .list_message_edits(b.messages[0].id, 100)
        .await
        .expect("edits");
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].body_after, "hello");
}

#[tokio::test]
async fn workspace_import_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(&SqliteStore::new(pool)).await;
}

#[tokio::test]
async fn workspace_import_postgres() {
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
    run_suite(&PostgresStore::new(pool)).await;
}
