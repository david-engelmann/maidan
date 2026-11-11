//! One delivery per event, however many times it is enqueued (CONC-003).
//!
//! Every replica runs the webhook and FSM hook workers, and every replica sees
//! every event, once live and again on a lag resume. The store is what keeps
//! that to one: one webhook delivery per subscription and event, one FSM hook
//! delivery per hook and event, one claim on a hook's run per event, and a
//! due delivery handed to one poller at a time.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AutomationSourceKind, FsmHookId, NewAutomationDelivery, NewFsmHook, NewWebhookSubscription,
    NewWorkspace, SlashHandlerKind, Workspace, WorkspaceId,
};

fn workspace_created() -> maidan_types::Event {
    maidan_types::Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: "logged".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    }
}

fn hook_delivery(ws: WorkspaceId, hook: FsmHookId, log_id: Option<i64>) -> NewAutomationDelivery {
    NewAutomationDelivery {
        workspace_id: ws,
        source_kind: AutomationSourceKind::FsmHook,
        source_id: hook.0,
        target_url: "https://example.test/fsm".into(),
        header_name: "X-Maidan-Event".into(),
        header_value: "thread_state_changed".into(),
        payload: "{}".into(),
        log_id,
    }
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "ha".into() })
        .await
        .expect("workspace")
        .id;
    let event = store
        .append_event(&workspace_created())
        .await
        .expect("append")
        .id;
    let later = store
        .append_event(&workspace_created())
        .await
        .expect("append later")
        .id;
    let subscription = |url: &'static str| NewWebhookSubscription {
        workspace_id: ws,
        url: url.into(),
        label: None,
        event_kinds: vec![],
        secret_ciphertext: "sealed".into(),
    };
    let sub = store
        .create_webhook_subscription(subscription("https://example.test/a"))
        .await
        .expect("subscription")
        .id;
    let other_sub = store
        .create_webhook_subscription(subscription("https://example.test/b"))
        .await
        .expect("other subscription")
        .id;

    // Webhooks: the second replica's enqueue of the same event adds nothing,
    // even a moment later (Postgres keys it on the event's time, not now).
    let first = store
        .enqueue_webhook_delivery(sub, event, "{}")
        .await
        .expect("enqueue");
    assert!(first.is_some(), "the first enqueue queues a delivery");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let again = store
        .enqueue_webhook_delivery(sub, event, "{}")
        .await
        .expect("enqueue again");
    assert_eq!(
        again, None,
        "the same event is queued once per subscription"
    );
    let pending = store
        .list_pending_webhook_deliveries(50)
        .await
        .expect("pending");
    assert_eq!(
        pending
            .iter()
            .filter(|d| d.subscription_id == sub && d.log_id == event)
            .count(),
        1
    );
    // The key is per subscription and per event.
    assert!(store
        .enqueue_webhook_delivery(other_sub, event, "{}")
        .await
        .expect("other subscription")
        .is_some());
    assert!(store
        .enqueue_webhook_delivery(sub, later, "{}")
        .await
        .expect("later event")
        .is_some());

    // FSM hook deliveries: one per hook and event. A slash command's delivery
    // names no event and is never folded into another.
    let hook = store
        .create_fsm_hook(NewFsmHook {
            workspace_id: ws,
            label: None,
            from_state: None,
            to_state: None,
            handler_kind: SlashHandlerKind::McpTool,
            handler_target: "clear_review_requirement".into(),
            secret_ciphertext: String::new(),
            created_by_token: None,
        })
        .await
        .expect("hook")
        .id;
    assert!(store
        .enqueue_automation_delivery(hook_delivery(ws, hook, Some(event)))
        .await
        .expect("hook delivery")
        .is_some());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        store
            .enqueue_automation_delivery(hook_delivery(ws, hook, Some(event)))
            .await
            .expect("hook delivery again"),
        None,
        "the same event is queued once per hook"
    );
    for _ in 0..2 {
        assert!(store
            .enqueue_automation_delivery(NewAutomationDelivery {
                source_kind: AutomationSourceKind::SlashCommand,
                ..hook_delivery(ws, hook, None)
            })
            .await
            .expect("slash delivery")
            .is_some());
    }
    assert_eq!(
        store
            .list_pending_automation_deliveries(50)
            .await
            .expect("pending automation")
            .len(),
        3
    );

    // A hook's run: one claim per event.
    assert!(store
        .claim_fsm_hook_run(ws, hook, event)
        .await
        .expect("claim"));
    assert!(!store
        .claim_fsm_hook_run(ws, hook, event)
        .await
        .expect("claim again"));
    assert!(store
        .claim_fsm_hook_run(ws, hook, later)
        .await
        .expect("claim later"));

    // Pollers: a claimed delivery is leased, so the next claim, another
    // replica's, gets a different row, and none once every row is held.
    let mut claimed = Vec::new();
    while let Some(d) = store
        .claim_next_webhook_delivery(120)
        .await
        .expect("claim webhook")
    {
        assert!(!claimed.contains(&d.id), "delivery {} claimed twice", d.id);
        claimed.push(d.id);
    }
    assert_eq!(claimed.len(), 3);
    assert!(store
        .list_pending_webhook_deliveries(50)
        .await
        .expect("pending after claims")
        .is_empty());
    let mut claimed = Vec::new();
    while let Some(d) = store
        .claim_next_automation_delivery(120)
        .await
        .expect("claim automation")
    {
        assert!(!claimed.contains(&d.id), "delivery {} claimed twice", d.id);
        claimed.push(d.id);
    }
    assert_eq!(claimed.len(), 3);
}

#[tokio::test]
async fn an_event_enqueued_twice_is_delivered_once_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn an_event_enqueued_twice_is_delivered_once_postgres() {
    use maidan_store::run_postgres_migrations;
    use sqlx::postgres::PgPoolOptions;
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
