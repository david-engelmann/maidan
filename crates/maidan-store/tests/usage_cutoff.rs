//! The hard allowance cutoff's store side, on both backends: a workspace's
//! origin is recorded by the sign-up transaction, and only an open sign-up
//! workspace's crossings are listed as cut off — never an invited or
//! operator-created one's, and never another month's.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn signed_up(store: &dyn Store, who: &str, open: bool) -> WorkspaceId {
    store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: Utc::now() + Duration::from_secs(3600),
                open,
            },
            Box::new(|core| NewAuditEvent {
                scope: AuditScope::Workspace(core.workspace.id),
                actor_id: Some(core.member.id),
                action: "workspace.signup".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(core.workspace.id.0),
                metadata: serde_json::json!({}),
            }),
            Box::new(|session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: serde_json::json!({}),
            }),
        )
        .await
        .unwrap()
        .workspace
        .id
}

async fn over(store: &dyn Store, ws: WorkspaceId, meter: UsageMeter, month: &str) {
    let mark = OverAllowance {
        workspace_id: ws,
        meter,
        month: month.into(),
        quantity: 11,
        allowance: 10,
        marked_at: Utc::now(),
    };
    store.mark_over_allowance(&mark, &[]).await.unwrap();
}

async fn run_suite(store: &dyn Store) {
    let open_a = signed_up(store, "ada", true).await;
    let open_b = signed_up(store, "bea", true).await;
    let invited = signed_up(store, "cy", false).await;
    let operator = store
        .create_workspace(NewWorkspace { name: "ops".into() })
        .await
        .unwrap()
        .id;

    assert!(
        store
            .list_usage_cut_off("2026-10")
            .await
            .unwrap()
            .is_empty(),
        "nothing is cut off before a crossing"
    );
    for ws in [open_a, invited, operator] {
        over(store, ws, UsageMeter::Requests, "2026-10").await;
    }
    over(store, open_b, UsageMeter::McpCalls, "2026-09").await;

    assert_eq!(
        store.list_usage_cut_off("2026-10").await.unwrap(),
        vec![(open_a, UsageMeter::Requests)],
        "only the open workspace past it, on its own meter; B's usage is B's"
    );
    assert_eq!(
        store.list_usage_cut_off("2026-09").await.unwrap(),
        vec![(open_b, UsageMeter::McpCalls)],
        "a month's crossings stay in that month"
    );
    assert!(store
        .list_usage_cut_off("2026-11")
        .await
        .unwrap()
        .is_empty());

    // Every meter, not just requests: past each one, the open workspace is
    // listed on each, and the invited and operator ones on none.
    for ws in [open_a, invited, operator] {
        for meter in UsageMeter::ALL {
            over(store, ws, meter, "2026-08").await;
        }
    }
    let mut listed = store.list_usage_cut_off("2026-08").await.unwrap();
    listed.sort_by_key(|(_, meter)| meter.as_str());
    let mut expected: Vec<_> = UsageMeter::ALL.into_iter().map(|m| (open_a, m)).collect();
    expected.sort_by_key(|(_, meter)| meter.as_str());
    assert_eq!(
        listed, expected,
        "each meter cuts off the open workspace only"
    );

    // A new open workspace signs up while another is cut off, and starts
    // with nothing cut off.
    let fresh = signed_up(store, "dee", true).await;
    assert!(store
        .list_usage_cut_off("2026-08")
        .await
        .unwrap()
        .iter()
        .all(|(ws, _)| *ws != fresh));
}

#[tokio::test]
async fn only_open_signup_workspaces_past_their_allowance_are_cut_off_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn only_open_signup_workspaces_past_their_allowance_are_cut_off_postgres() {
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
