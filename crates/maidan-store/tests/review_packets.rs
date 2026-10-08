//! Review packets on both backends. Each start_review records what the thread
//! put in front of its reviewers (its version, its result's hash and its linked
//! artifacts) and the root of that manifest; a later hand-off writes a new
//! packet; no packet can be updated.

use std::time::Duration;

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const HELD: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

/// Runs the scenario and returns the first packet's id, for the backend's own
/// immutability check.
async fn run_suite(store: &dyn Store) -> uuid::Uuid {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "packets".into(),
        })
        .await
        .expect("workspace")
        .id;
    let (worker, reviewer) = {
        let mut ids = Vec::new();
        for handle in ["worker", "reviewer"] {
            ids.push(
                store
                    .create_member(NewMember {
                        workspace_id: ws,
                        handle: handle.into(),
                        display_name: None,
                        kind: MemberKind::Agent,
                    })
                    .await
                    .expect("member")
                    .id,
            );
        }
        (ids[0], ids[1])
    };
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let thread = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
        })
        .await
        .expect("thread")
        .id;
    assert!(store
        .latest_review_packet(thread)
        .await
        .expect("none yet")
        .is_none());

    store.claim_thread(thread, worker).await.expect("claim");
    let result = json!({"status": "done", "files": ["a.rs"]});
    store
        .set_thread_result(thread, worker, &result)
        .await
        .expect("result");
    store.record_artifact_ref(ws, HELD).await.expect("held");
    store
        .link_thread_artifact(thread, HELD, worker)
        .await
        .expect("link");
    store
        .transition_thread(thread, worker, ThreadAction::StartReview)
        .await
        .expect("start review");

    let first = store
        .latest_review_packet(thread)
        .await
        .expect("read")
        .expect("a packet");
    assert_eq!(first.requested_by, worker);
    assert_eq!(
        first.manifest,
        EvidenceManifest {
            thread_id: thread,
            thread_version: store.thread_version(thread).await.expect("version"),
            result: Some(ResultEvidence {
                sha256: result_sha256(&result).expect("hash"),
                produced_by: worker,
            }),
            artifacts: vec![HELD.to_string()],
        }
    );
    assert_eq!(
        first.evidence_root,
        first.manifest.root().expect("root"),
        "the stored root is the manifest's"
    );

    // Sent back, reworked and handed over again: a new packet, a new root,
    // and the first one kept as it was.
    store
        .submit_review(
            thread,
            reviewer,
            ReviewDecision::RequestChanges,
            Some("cover the empty list"),
        )
        .await
        .expect("send back");
    store
        .set_thread_result(
            thread,
            worker,
            &json!({"status": "done", "files": ["a.rs", "b.rs"]}),
        )
        .await
        .expect("rework");
    store
        .transition_thread(thread, worker, ThreadAction::StartReview)
        .await
        .expect("review again");
    let second = store
        .latest_review_packet(thread)
        .await
        .expect("read")
        .expect("a packet");
    assert_ne!(second.id, first.id);
    assert!(second.manifest.thread_version > first.manifest.thread_version);
    assert_ne!(second.evidence_root, first.evidence_root);

    first.id
}

#[tokio::test]
async fn every_hand_off_to_review_records_an_immutable_packet_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let first = run_suite(&SqliteStore::for_tests(pool.clone())).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM maidan_review_packets")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 2, "both hand-offs are kept");
    let refused = sqlx::query("UPDATE maidan_review_packets SET evidence_root = 'x' WHERE id = ?")
        .bind(first)
        .execute(&pool)
        .await;
    assert!(
        refused.is_err_and(|e| e.to_string().contains("immutable")),
        "a packet cannot be rewritten"
    );
}

#[tokio::test]
async fn every_hand_off_to_review_records_an_immutable_packet_on_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let first = run_suite(&PostgresStore::for_tests(pool.clone())).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM maidan_review_packets")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 2, "both hand-offs are kept");
    let refused = sqlx::query("UPDATE maidan_review_packets SET evidence_root = 'x' WHERE id = $1")
        .bind(first)
        .execute(&pool)
        .await;
    assert!(
        refused.is_err_and(|e| e.to_string().contains("immutable")),
        "a packet cannot be rewritten"
    );
}
