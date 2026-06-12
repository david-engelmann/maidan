//! Bulk context reads (Cluster 106.0.1): the batched thread / reference / edit
//! accessors return the same rows as the per-row reads they replace, respect
//! the per-message edit limit, and behave on empty input — identically on both
//! backends.

use std::time::Duration;

use maidan_store::{
    run_postgres_migrations, run_sqlite_migrations, PostgresStore, SqliteStore, Store,
};
use maidan_types::*;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn assert_bulk_reads(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "bulk".into(),
        })
        .await
        .unwrap();
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "author".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();

    // Two channels with threads spread across them.
    let mut channels = Vec::new();
    for name in ["c1", "c2"] {
        channels.push(
            store
                .create_channel(NewChannel {
                    workspace_id: ws.id,
                    name: name.into(),
                    topic: None,
                    private: false,
                })
                .await
                .unwrap(),
        );
    }
    let mut threads = Vec::new();
    for channel in [channels[0].id, channels[0].id, channels[1].id] {
        threads.push(
            store
                .create_thread(NewThread {
                    channel_id: channel,
                    parent_thread_id: None,
                    title: None,
                })
                .await
                .unwrap(),
        );
    }
    let (t1, t2, t3) = (threads[0].clone(), threads[1].clone(), threads[2].clone());

    // list_threads_for_workspace == union of the per-channel lists.
    let all = store.list_threads_for_workspace(ws.id).await.unwrap();
    let mut got: Vec<_> = all.iter().map(|t| t.id.0).collect();
    got.sort();
    let mut expect = vec![t1.id.0, t2.id.0, t3.id.0];
    expect.sort();
    assert_eq!(got, expect, "all workspace threads in one read");

    // Messages + references from them (m1 → 2 refs, m2 → 1 ref).
    let mut messages = Vec::new();
    for body in ["m1", "m2"] {
        messages.push(
            store
                .post_message(NewMessage {
                    thread_id: t1.id,
                    author_id: author.id,
                    body: body.into(),
                    metadata: json!({}),
                })
                .await
                .unwrap(),
        );
    }
    let (m1, m2) = (messages[0].clone(), messages[1].clone());
    for (src, dst, rel) in [
        (m1.id.0, t2.id.0, "rel-a"),
        (m1.id.0, t3.id.0, "rel-b"),
        (m2.id.0, t2.id.0, "rel-c"),
    ] {
        store
            .add_reference(NewReference {
                src_kind: RefSide::Message,
                src_id: src,
                dst_kind: RefSide::Thread,
                dst_id: dst,
                relation: rel.into(),
            })
            .await
            .unwrap();
    }

    let many = store
        .list_references_from_many(RefSide::Message, &[m1.id.0, m2.id.0])
        .await
        .unwrap();
    assert_eq!(many.len(), 3);
    assert_eq!(
        many.iter().filter(|r| r.src_id == m1.id.0).count(),
        store
            .list_references_from(RefSide::Message, m1.id.0)
            .await
            .unwrap()
            .len(),
        "batched references for m1 match the single-source read"
    );
    assert_eq!(many.iter().filter(|r| r.src_id == m2.id.0).count(), 1);
    assert!(store
        .list_references_from_many(RefSide::Message, &[])
        .await
        .unwrap()
        .is_empty());

    // Edits: m1 gets 3, m2 gets 1.
    for body in ["e1", "e2", "e3"] {
        store
            .edit_message(
                m1.id,
                author.id,
                EditMessage {
                    body: body.into(),
                    metadata: json!({}),
                },
            )
            .await
            .unwrap();
    }
    store
        .edit_message(
            m2.id,
            author.id,
            EditMessage {
                body: "e".into(),
                metadata: json!({}),
            },
        )
        .await
        .unwrap();

    // limit_per caps each message independently (m1 → 2 of 3, m2 → its 1).
    let limited = store
        .list_message_edits_for_messages(&[m1.id, m2.id], 2)
        .await
        .unwrap();
    assert_eq!(limited.iter().filter(|e| e.message_id == m1.id).count(), 2);
    assert_eq!(limited.iter().filter(|e| e.message_id == m2.id).count(), 1);

    // A high limit returns all of m1's edits, in the same order as the single read.
    let bulk_m1 = store
        .list_message_edits_for_messages(&[m1.id], 20)
        .await
        .unwrap();
    let single_m1 = store.list_message_edits(m1.id, 20).await.unwrap();
    assert_eq!(bulk_m1.len(), 3);
    assert_eq!(
        bulk_m1.iter().map(|e| e.id).collect::<Vec<_>>(),
        single_m1.iter().map(|e| e.id).collect::<Vec<_>>(),
        "batched edits match single-read order"
    );
    assert!(store
        .list_message_edits_for_messages(&[], 20)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn bulk_reads_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);
    assert_bulk_reads(&store).await;
}

#[tokio::test]
async fn bulk_reads_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping bulk_reads postgres: docker unavailable ({err})");
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
    assert_bulk_reads(&store).await;
}
