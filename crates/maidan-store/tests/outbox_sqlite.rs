//! SQLite outbox integration tests.

use chrono::Utc;
use maidan_store::{prelude::*, run_sqlite_migrations, sqlite::outbox, OutboxRow};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite_pool() -> sqlx::SqlitePool {
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
    pool
}

fn workspace_created_event(name: &str) -> Event {
    Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: name.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    }
}

#[tokio::test]
async fn append_enqueues_unpublished_outbox_row() {
    let pool = sqlite_pool().await;
    let store = SqliteStore::for_tests(pool.clone());
    store
        .append_event(&workspace_created_event("sqlite-outbox-ws"))
        .await
        .unwrap();
    assert!(outbox::count_pending(&pool).await.unwrap() >= 1);
}

#[tokio::test]
async fn quarantined_rows_are_excluded_from_pending_list_and_count() {
    let pool = sqlite_pool().await;
    let store = SqliteStore::for_tests(pool.clone());
    store
        .append_event(&workspace_created_event("sqlite-quarantine"))
        .await
        .unwrap();
    let pending = outbox::list_pending(&pool, &ContentKeyring::insecure_dev(), 1)
        .await
        .unwrap();
    outbox::quarantine(&pool, pending[0].id).await.unwrap();
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);
    assert_eq!(outbox::count_quarantined(&pool).await.unwrap(), 1);
}

/// The SQLite twin of the claim. SQLite serializes writers, so a second claimer
/// sees the first's committed `claimed_at` and skips the row — and an expired
/// lease is reclaimable so a crashed relay strands nothing.
#[tokio::test]
async fn a_claimed_row_is_excluded_until_its_lease_expires() {
    let pool = sqlite_pool().await;
    let store = SqliteStore::for_tests(pool.clone());
    store
        .append_event(&workspace_created_event("sqlite-claim"))
        .await
        .unwrap();

    let first = outbox::claim_pending(&pool, &ContentKeyring::insecure_dev(), 8, 60)
        .await
        .unwrap();
    assert_eq!(first.len(), 1, "the row is claimable once");

    assert!(
        outbox::claim_pending(&pool, &ContentKeyring::insecure_dev(), 8, 60)
            .await
            .unwrap()
            .is_empty(),
        "a live claim must exclude the row from a second relay"
    );

    // A failed attempt releases the claim for a prompt retry.
    outbox::record_attempt(&pool, first[0].id).await.unwrap();
    assert_eq!(
        outbox::claim_pending(&pool, &ContentKeyring::insecure_dev(), 8, 60)
            .await
            .unwrap()
            .len(),
        1,
        "record_attempt should release the claim"
    );

    // An expired lease is reclaimable.
    assert_eq!(
        outbox::claim_pending(&pool, &ContentKeyring::insecure_dev(), 8, 0)
            .await
            .unwrap()
            .len(),
        1,
        "an expired claim must be reclaimable"
    );
}

/// The relay publishes a message's words from the claim: the row carries the
/// live content key beside the sealed payload. Once the message is withdrawn
/// the key is gone, and the row publishes only the redacted state.
#[tokio::test]
async fn claimed_rows_open_live_words_and_not_shredded_ones() {
    let pool = sqlite_pool().await;
    let keys = std::sync::Arc::new(ContentKeyring::new([7; 32], Vec::new()));
    let store = SqliteStore::new(pool.clone(), keys.clone());
    let ws = store
        .create_workspace(NewWorkspace {
            name: "relay".into(),
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
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let (message, posted) = store
        .post_message_with_event(
            NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: "relay words".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();

    let body_of = |row: &OutboxRow| match BusEnvelope::from_sealed_payload(
        row.log_id,
        row.payload.clone(),
        row.content_key.as_ref(),
    )
    .unwrap()
    .event
    {
        Event::MessagePosted { message, .. } => message.body,
        other => panic!("expected MessagePosted, got {other:?}"),
    };
    let pending = outbox::list_pending(&pool, &keys, 100).await.unwrap();
    let row = pending.iter().find(|r| r.log_id == posted.id).unwrap();
    assert!(!row.payload.to_string().contains("relay words"));
    assert_eq!(body_of(row), "relay words");

    store
        .tombstone_message_with_event(message.id, None)
        .await
        .unwrap();
    let pending = outbox::list_pending(&pool, &keys, 100).await.unwrap();
    let row = pending.iter().find(|r| r.log_id == posted.id).unwrap();
    assert!(row.content_key.is_none());
    assert_eq!(body_of(row), "");
}
