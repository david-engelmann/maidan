//! Attachable labeled memory-block store (Cluster 373, Wave 2 #21, H11): create
//! (concurrent-safe on label), get/list, full-rewrite set_value (last-writer-
//! wins), read-only + over-limit refusal, attach/detach (idempotent), and the
//! thread-attached list. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMemoryBlock, NewThread, NewWorkspace};
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

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("owner");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .expect("thread");

    // Create a block.
    let block = store
        .create_memory_block(NewMemoryBlock {
            workspace_id: ws.id,
            label: "shared".into(),
            description: Some("the shared scratchpad".into()),
            char_limit: Some(10),
            read_only: false,
            value: "hello".into(),
            owner_id: owner.id,
        })
        .await
        .expect("create");
    assert_eq!(block.label, "shared");
    assert_eq!(block.value, "hello");
    assert_eq!(block.char_limit, Some(10));

    // Create is concurrent-safe on (workspace, label): re-creating the label
    // returns the SAME block (not an error, not a duplicate).
    let again = store
        .create_memory_block(NewMemoryBlock {
            workspace_id: ws.id,
            label: "shared".into(),
            description: None,
            char_limit: Some(999),
            read_only: false,
            value: "different".into(),
            owner_id: owner.id,
        })
        .await
        .expect("recreate");
    assert_eq!(again.id, block.id, "same label converges on one block");
    assert_eq!(again.value, "hello", "existing value preserved");

    // Read by id + by label + list.
    assert_eq!(
        store.get_memory_block(block.id).await.unwrap().unwrap().id,
        block.id
    );
    assert_eq!(
        store
            .get_memory_block_by_label(ws.id, "shared")
            .await
            .unwrap()
            .unwrap()
            .id,
        block.id
    );
    assert_eq!(store.list_memory_blocks(ws.id).await.unwrap().len(), 1);

    // set_value is a full rewrite (last-writer-wins).
    let updated = store
        .set_memory_block_value(block.id, "world")
        .await
        .expect("set");
    assert_eq!(updated.value, "world");
    assert!(updated.updated_at >= block.updated_at);

    // Over the char limit is refused (InvalidInput).
    let too_long = store
        .set_memory_block_value(block.id, "this is way too long")
        .await;
    assert!(matches!(too_long, Err(StoreError::InvalidInput(_))));
    // The value was NOT changed.
    assert_eq!(
        store
            .get_memory_block(block.id)
            .await
            .unwrap()
            .unwrap()
            .value,
        "world"
    );

    // A read-only block refuses writes.
    let ro = store
        .create_memory_block(NewMemoryBlock {
            workspace_id: ws.id,
            label: "frozen".into(),
            description: None,
            char_limit: None,
            read_only: true,
            value: "immutable".into(),
            owner_id: owner.id,
        })
        .await
        .expect("ro");
    let refused = store.set_memory_block_value(ro.id, "nope").await;
    assert!(matches!(refused, Err(StoreError::InvalidInput(_))));

    // Attach to a thread (idempotent).
    assert!(store
        .attach_memory_block(thread.id, block.id)
        .await
        .unwrap());
    assert!(
        !store
            .attach_memory_block(thread.id, block.id)
            .await
            .unwrap(),
        "re-attach is a no-op"
    );
    store
        .attach_memory_block(thread.id, ro.id)
        .await
        .expect("attach ro");
    let attached = store.list_thread_memory_blocks(thread.id).await.unwrap();
    assert_eq!(attached.len(), 2);
    // Ordered by label: "frozen" before "shared".
    assert_eq!(attached[0].label, "frozen");
    assert_eq!(attached[1].label, "shared");

    // Detach (idempotent).
    assert!(store.detach_memory_block(thread.id, ro.id).await.unwrap());
    assert!(
        !store.detach_memory_block(thread.id, ro.id).await.unwrap(),
        "re-detach is a no-op"
    );
    assert_eq!(
        store
            .list_thread_memory_blocks(thread.id)
            .await
            .unwrap()
            .len(),
        1
    );

    // Delete a block.
    assert!(store.delete_memory_block(ro.id).await.unwrap());
    assert!(store.get_memory_block(ro.id).await.unwrap().is_none());
    // set_value on an unknown block is NotFound.
    assert!(matches!(
        store.set_memory_block_value(ro.id, "x").await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn memory_blocks_crud_attach_and_rewrite_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn memory_blocks_crud_attach_and_rewrite_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
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
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
