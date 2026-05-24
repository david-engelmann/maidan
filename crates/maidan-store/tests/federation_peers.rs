//! Federation peer store: CRUD, token lookup, ingest dedupe.

use maidan_auth::{decrypt_peer_secret, encrypt_peer_secret};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewMember, NewPeer, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

async fn seed_workspace(store: &dyn Store) -> maidan_types::WorkspaceId {
    store
        .create_workspace(NewWorkspace {
            name: "fed-ws".to_string(),
        })
        .await
        .expect("workspace")
        .id
}

#[tokio::test]
async fn peer_create_lookup_and_delete() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = seed_workspace(&store).await;

    let peer = store
        .create_peer(NewPeer {
            workspace_id: ws,
            remote_workspace_id: ws,
            name: "east".to_string(),
            base_url: "https://east.example".to_string(),
            token_hash: "a".repeat(64),
            outbound_secret_ciphertext: None,
        })
        .await
        .expect("create peer");

    let fetched = store.get_peer(peer.id).await.expect("get");
    assert_eq!(fetched.name, "east");
    assert!(!fetched.token_hash.is_empty());

    store.delete_peer(peer.id).await.expect("delete");
    store.get_peer(peer.id).await.expect_err("gone");
}

#[tokio::test]
async fn peer_token_hash_is_unique() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = seed_workspace(&store).await;
    let hash = "b".repeat(64);

    store
        .create_peer(NewPeer {
            workspace_id: ws,
            remote_workspace_id: ws,
            name: "p1".to_string(),
            base_url: "https://p1.example".to_string(),
            token_hash: hash.clone(),
            outbound_secret_ciphertext: None,
        })
        .await
        .expect("first");

    let err = store
        .create_peer(NewPeer {
            workspace_id: ws,
            remote_workspace_id: ws,
            name: "p2".to_string(),
            base_url: "https://p2.example".to_string(),
            token_hash: hash,
            outbound_secret_ciphertext: None,
        })
        .await
        .expect_err("duplicate hash");
    assert!(matches!(err, maidan_store::StoreError::Conflict(_)));
}

#[tokio::test]
async fn federated_ingest_dedupes_by_peer_and_remote_id() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = seed_workspace(&store).await;
    let peer = store
        .create_peer(NewPeer {
            workspace_id: ws,
            remote_workspace_id: ws,
            name: "upstream".to_string(),
            base_url: "https://up.example".to_string(),
            token_hash: "c".repeat(64),
            outbound_secret_ciphertext: None,
        })
        .await
        .expect("peer");

    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "bot".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");

    let event = store
        .append_event(&maidan_types::Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member: member.clone(),
        })
        .await
        .expect("append");

    let first = store
        .try_record_federated_ingest(peer.id, 99, event.id)
        .await
        .expect("record");
    assert!(first);
    let dup = store
        .try_record_federated_ingest(peer.id, 99, event.id)
        .await
        .expect("record again");
    assert!(!dup);

    assert!(store
        .is_federated_local_event(event.id)
        .await
        .expect("check"));
    assert!(!store
        .is_federated_local_event(event.id + 9999)
        .await
        .expect("check"));
}

#[tokio::test]
async fn update_peer_cursor_advances_last_synced_event_id() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = seed_workspace(&store).await;
    let peer = store
        .create_peer(NewPeer {
            workspace_id: ws,
            remote_workspace_id: ws,
            name: "cursor-peer".to_string(),
            base_url: "https://cursor.example".to_string(),
            token_hash: "d".repeat(64),
            outbound_secret_ciphertext: None,
        })
        .await
        .expect("peer");

    let updated = store.update_peer_cursor(peer.id, 42).await.expect("update");
    assert_eq!(updated.last_synced_event_id, 42);
}

#[tokio::test]
async fn peer_outbound_secret_ciphertext_round_trips_via_auth() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = seed_workspace(&store).await;
    let key = [0xab; 32];
    let plaintext = "outbound-bearer-secret";
    let ciphertext = encrypt_peer_secret(plaintext, &key).expect("encrypt");

    let peer = store
        .create_peer(NewPeer {
            workspace_id: ws,
            remote_workspace_id: ws,
            name: "encrypted".to_string(),
            base_url: "https://enc.example".to_string(),
            token_hash: "e".repeat(64),
            outbound_secret_ciphertext: Some(ciphertext.clone()),
        })
        .await
        .expect("create");

    let loaded = store.get_peer(peer.id).await.expect("get");
    assert_eq!(
        loaded.outbound_secret_ciphertext.as_deref(),
        Some(ciphertext.as_str())
    );
    assert_eq!(
        decrypt_peer_secret(loaded.outbound_secret_ciphertext.as_ref().unwrap(), &key)
            .expect("decrypt"),
        plaintext
    );
}
