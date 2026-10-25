//! `list_workspace_usage_ledger`: a workspace's completed usage reports,
//! newest first and never another workspace's. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewThread, NewUsageLedgerEntry, NewWorkspace, PriceSnapshot,
    TokenUsage, UsageEvidence, WorkspaceId,
};
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
    SqliteStore::for_tests(pool)
}

/// A workspace with one agent that reports `reports` calls on one thread,
/// each tagged with its index as the harness version.
async fn workspace_with_reports(store: &dyn Store, name: &str, reports: usize) -> WorkspaceId {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-agent"),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: format!("{name}-work"),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("fleet task".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    let claimed = store
        .claim_thread(thread.id, member.id)
        .await
        .expect("claim");
    let lease = claimed.thread.claim_lease_id.expect("claim lease");
    for i in 0..reports {
        store
            .report_accounted_usage(&NewUsageLedgerEntry {
                usage_report_id: uuid::Uuid::now_v7(),
                thread_id: thread.id,
                reporter: member.id,
                claim_lease_id: lease,
                model: "provider/model".into(),
                tokens: TokenUsage {
                    input: 1,
                    ..Default::default()
                },
                usd_micros: 0,
                price_snapshot: PriceSnapshot::default(),
                turns: 1,
                evidence: UsageEvidence {
                    harness: Some(format!("{name}-harness")),
                    harness_version: Some(i.to_string()),
                    ..Default::default()
                },
            })
            .await
            .expect("report");
    }
    ws.id
}

async fn run_suite(store: &dyn Store) {
    let a = workspace_with_reports(store, "fleet-a", 3).await;
    let b = workspace_with_reports(store, "fleet-b", 2).await;

    let listed = store
        .list_workspace_usage_ledger(a, 10)
        .await
        .expect("list a");
    assert_eq!(listed.len(), 3);
    assert!(
        listed.iter().all(|e| e.stamp.payer == a
            && e.evidence.harness.as_deref() == Some("fleet-a-harness")),
        "only A's reports"
    );
    assert!(
        listed
            .windows(2)
            .all(|w| (w[0].accepted_at, w[0].usage_report_id)
                >= (w[1].accepted_at, w[1].usage_report_id)),
        "newest first"
    );

    let capped = store
        .list_workspace_usage_ledger(a, 2)
        .await
        .expect("capped");
    assert_eq!(capped.len(), 2);
    assert_eq!(capped[0].usage_report_id, listed[0].usage_report_id);

    let other = store
        .list_workspace_usage_ledger(b, 10)
        .await
        .expect("list b");
    assert_eq!(other.len(), 2);
    assert!(other.iter().all(|e| e.stamp.payer == b));
}

#[tokio::test]
async fn a_workspace_lists_only_its_own_reports_newest_first_on_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_workspace_lists_only_its_own_reports_newest_first_on_postgres() {
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
