//! Concurrent accepts of one intake item into a channel, on both backends:
//! exactly one wins, the rest get `Conflict`, and only the winner's thread
//! exists. The thread is created inside the triage transaction, so a losing
//! accept leaves no orphan thread behind.

use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations, StoreError};
use maidan_types::{
    AuditScope, IntakeDecision, IntakeSource, IntakeState, MemberKind, NewAuditEvent, NewChannel,
    NewIntakeItem, NewMember, NewThread, NewWorkspace,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const RACERS: usize = 8;

async fn run_race(store: Arc<dyn Store>) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "intake-race".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "triager".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "intake".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let (item, _) = store
        .create_intake_item(NewIntakeItem {
            workspace_id: ws.id,
            title: "race me".into(),
            body: None,
            source: IntakeSource::Human,
            proposed_by: member.id,
        })
        .await
        .unwrap();

    let mut racers = Vec::new();
    for _ in 0..RACERS {
        let store = store.clone();
        racers.push(tokio::spawn(async move {
            store
                .triage_intake_item_audited(
                    ws.id,
                    item.id,
                    member.id,
                    IntakeDecision::AcceptNewThread {
                        thread: NewThread {
                            channel_id: channel.id,
                            parent_thread_id: None,
                            title: Some("race me".into()),
                            description: None,
                            created_by: Some(member.id),
                        },
                    },
                    NewAuditEvent {
                        scope: AuditScope::Workspace(ws.id),
                        actor_id: None,
                        action: "intake.accept".into(),
                        target_kind: Some("intake_item".into()),
                        target_id: Some(item.id),
                        metadata: serde_json::json!({ "channel_id": channel.id.0 }),
                    },
                )
                .await
        }));
    }
    let mut winners = Vec::new();
    for racer in racers {
        match racer.await.unwrap() {
            Ok((won, events)) => {
                assert_eq!(events.len(), 2, "ThreadCreated then IntakeAccepted");
                winners.push(won);
            }
            Err(StoreError::Conflict(_)) => {}
            Err(other) => panic!("a losing accept must be a Conflict, got {other:?}"),
        }
    }
    assert_eq!(winners.len(), 1, "exactly one accept wins");
    let won = &winners[0];
    assert_eq!(won.state, IntakeState::Accepted);

    let threads = store.list_threads(channel.id).await.unwrap();
    assert_eq!(threads.len(), 1, "no orphan threads from losing accepts");
    assert_eq!(Some(threads[0].id), won.thread_id);
}

#[tokio::test]
async fn concurrent_accepts_leave_one_thread_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(dir.path().join("race.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(30))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(RACERS as u32)
        .connect_with(options)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_race(Arc::new(SqliteStore::for_tests(pool))).await;
}

#[tokio::test]
async fn concurrent_accepts_leave_one_thread_postgres() {
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
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(RACERS as u32)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    run_race(Arc::new(PostgresStore::for_tests(pool))).await;
}
