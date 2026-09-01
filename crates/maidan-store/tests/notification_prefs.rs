//! Notification preferences (Cluster 241, Arc H): set (upsert) a mute flag per
//! event kind, list, and the router's `is_muted` query. Both backends. No wiring yet.

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{EventKind, MemberKind, NewMember, NewWorkspace};
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
        .create_workspace(NewWorkspace {
            name: "prefs".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "m".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");

    // Default: nothing muted.
    assert!(!store
        .is_notification_muted(member.id, EventKind::MentionRecorded)
        .await
        .expect("is_muted default"));
    assert!(store
        .list_notification_prefs(member.id)
        .await
        .expect("list empty")
        .is_empty());

    // Mute mentions.
    let pref = store
        .set_notification_pref(member.id, EventKind::MentionRecorded, true)
        .await
        .expect("set mute");
    assert_eq!(pref.kind, EventKind::MentionRecorded);
    assert!(pref.muted);
    assert!(store
        .is_notification_muted(member.id, EventKind::MentionRecorded)
        .await
        .expect("is_muted true"));
    // A different kind is still not muted.
    assert!(!store
        .is_notification_muted(member.id, EventKind::MessagePosted)
        .await
        .expect("other kind"));

    // A re-set overwrites (unmute).
    store
        .set_notification_pref(member.id, EventKind::MentionRecorded, false)
        .await
        .expect("unmute");
    assert!(!store
        .is_notification_muted(member.id, EventKind::MentionRecorded)
        .await
        .expect("is_muted after unmute"));
    // The row persists (explicit not-muted), listable.
    let prefs = store
        .list_notification_prefs(member.id)
        .await
        .expect("list");
    assert_eq!(prefs.len(), 1);
    assert!(!prefs[0].muted);

    // Cluster 348: the batch mute-filter returns exactly the members who muted the
    // kind, out of the given set. `member` has MessagePosted un-set (→ not muted);
    // a second member mutes it; a third has no prefs.
    let m2 = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "m2".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("m2");
    let m3 = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "m3".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("m3");
    store
        .set_notification_pref(m2.id, EventKind::MessagePosted, true)
        .await
        .expect("mute m2 message_posted");
    let muted = store
        .filter_muted_members(EventKind::MessagePosted, &[member.id, m2.id, m3.id])
        .await
        .expect("filter_muted");
    assert_eq!(muted, vec![m2.id], "only m2 muted MessagePosted");
    // A different kind: none of them muted it.
    assert!(store
        .filter_muted_members(EventKind::ThreadReady, &[member.id, m2.id, m3.id])
        .await
        .expect("filter_muted other kind")
        .is_empty());
    // Empty input → empty (no query).
    assert!(store
        .filter_muted_members(EventKind::MessagePosted, &[])
        .await
        .expect("filter_muted empty")
        .is_empty());
}

#[tokio::test]
async fn notification_prefs_set_list_is_muted_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn notification_prefs_set_list_is_muted_postgres() {
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
