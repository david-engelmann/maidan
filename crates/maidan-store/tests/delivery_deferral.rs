//! Deferring a delivery the retry budget held back: the webhook, automation,
//! mail and egress queues each move the row's next attempt forward and leave
//! its attempt count and last error as they were, so a deferral is never a
//! step toward the dead-letter queue. Both backends.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations, AutomationDeliveryFilter};
use maidan_types::{
    AutomationSourceKind, EgressKind, EgressTarget, NewAutomationDelivery, NewChannel,
    NewEgressOutbox, NewFsmHook, NewMailOutbox, NewThread, NewWebhookSubscription, NewWorkspace,
    SlashHandlerKind, ThreadState,
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

async fn webhook_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "hooks".into(),
        })
        .await
        .expect("ws");
    let sub = store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws.id,
            url: "https://hooks.example.com/in".into(),
            label: None,
            event_kinds: vec!["message_posted".into()],
            secret_ciphertext: "x".into(),
        })
        .await
        .expect("sub");
    let id = store
        .enqueue_webhook_delivery(sub.id, 1, "{}")
        .await
        .expect("enqueue");
    let attempts = store
        .record_webhook_delivery_attempt(id, "HTTP 503", Utc::now() - Duration::seconds(1))
        .await
        .expect("attempt");
    assert_eq!(attempts, 1);

    store
        .defer_webhook_delivery(id, Utc::now() + Duration::seconds(30))
        .await
        .expect("defer");
    let pending = store
        .list_pending_webhook_deliveries(10)
        .await
        .expect("list");
    assert!(
        pending.iter().all(|d| d.id != id),
        "a deferred delivery is not due"
    );
    let row = store.get_webhook_delivery(id, ws.id).await.expect("get");
    assert_eq!(row.attempts, 1, "a deferral is not an attempt");
    assert_eq!(row.last_error.as_deref(), Some("HTTP 503"));
    assert!(row.quarantined_at.is_none());

    store
        .defer_webhook_delivery(id, Utc::now() - Duration::seconds(1))
        .await
        .expect("defer again");
    let pending = store
        .list_pending_webhook_deliveries(10)
        .await
        .expect("list");
    let due = pending.iter().find(|d| d.id == id).expect("due again");
    assert_eq!(due.attempts, 1);

    store.quarantine_webhook_delivery(id).await.expect("dlq");
    store
        .defer_webhook_delivery(id, Utc::now() - Duration::seconds(1))
        .await
        .expect("defer quarantined");
    assert!(
        store
            .get_webhook_delivery(id, ws.id)
            .await
            .expect("get")
            .quarantined_at
            .is_some(),
        "deferring does not pull a row out of the DLQ"
    );
}

async fn automation_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "auto".into(),
        })
        .await
        .expect("ws");
    let hook = store
        .create_fsm_hook(NewFsmHook {
            workspace_id: ws.id,
            label: None,
            from_state: Some(ThreadState::Open),
            to_state: Some(ThreadState::InReview),
            handler_kind: SlashHandlerKind::Http,
            handler_target: "https://hooks.example.com/fsm".into(),
            secret_ciphertext: "x".into(),
        })
        .await
        .expect("hook");
    let id = store
        .enqueue_automation_delivery(NewAutomationDelivery {
            workspace_id: ws.id,
            source_kind: AutomationSourceKind::FsmHook,
            source_id: hook.id.0,
            target_url: "https://hooks.example.com/fsm".into(),
            header_name: "X-Maidan-Event".into(),
            header_value: "thread_state_changed".into(),
            payload: "{}".into(),
        })
        .await
        .expect("enqueue");
    store
        .record_automation_delivery_attempt(id, "HTTP 503", Utc::now() - Duration::seconds(1))
        .await
        .expect("attempt");

    store
        .defer_automation_delivery(id, Utc::now() + Duration::seconds(30))
        .await
        .expect("defer");
    let pending = store
        .list_pending_automation_deliveries(10)
        .await
        .expect("list");
    assert!(
        pending.iter().all(|d| d.id != id),
        "a deferred delivery is not due"
    );
    let row = store.get_automation_delivery(id, ws.id).await.expect("get");
    assert_eq!(row.attempts, 1, "a deferral is not an attempt");
    assert_eq!(row.last_error.as_deref(), Some("HTTP 503"));
    let still_pending = store
        .list_automation_deliveries(ws.id, AutomationDeliveryFilter::Pending, 10)
        .await
        .expect("pending view");
    assert!(still_pending.iter().any(|d| d.id == id));

    store
        .defer_automation_delivery(id, Utc::now() - Duration::seconds(1))
        .await
        .expect("defer again");
    let pending = store
        .list_pending_automation_deliveries(10)
        .await
        .expect("list");
    assert_eq!(
        pending
            .iter()
            .find(|d| d.id == id)
            .expect("due again")
            .attempts,
        1
    );
}

async fn mail_suite(store: &dyn Store) {
    let id = store
        .enqueue_mail(NewMailOutbox {
            workspace_id: None,
            source_log_id: None,
            to_address: "a@example.com".into(),
            subject: "s".into(),
            body: "b".into(),
        })
        .await
        .expect("enqueue")
        .expect("queued");
    store
        .claim_next_due_mail(Utc::now(), 300)
        .await
        .expect("claim")
        .expect("first claim");
    store
        .mark_mail_failed(id, "smtp down", Some(Utc::now() - Duration::seconds(1)))
        .await
        .expect("fail");
    let retry = store
        .claim_next_due_mail(Utc::now(), 300)
        .await
        .expect("claim")
        .expect("retry claim");
    assert_eq!(retry.attempts, 2);

    // The retry is held back: the claim's attempt is given back.
    store
        .defer_mail(id, Utc::now() + Duration::seconds(30))
        .await
        .expect("defer");
    assert!(
        store
            .claim_next_due_mail(Utc::now(), 300)
            .await
            .expect("claim")
            .is_none(),
        "a deferred entry is not due"
    );
    let later = store
        .claim_next_due_mail(Utc::now() + Duration::seconds(31), 300)
        .await
        .expect("claim")
        .expect("due after the deferral");
    assert_eq!(later.attempts, 2, "the deferred claim did not count");

    store
        .mark_mail_failed(id, "gave up", None)
        .await
        .expect("dead");
    store
        .defer_mail(id, Utc::now() - Duration::seconds(1))
        .await
        .expect("defer dead");
    assert_eq!(store.count_dead_mail().await.expect("count"), 1);
    assert!(
        store
            .claim_next_due_mail(Utc::now(), 300)
            .await
            .expect("claim")
            .is_none(),
        "deferring does not revive a dead-lettered entry"
    );
}

async fn egress_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "egress".into(),
        })
        .await
        .expect("ws");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("egress".into()),
        })
        .await
        .expect("thread");
    let id = store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: ws.id,
            thread_id: thread.id,
            source_log_id: 1,
            target: EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into(),
            },
            body: "hi".into(),
            kind: EgressKind::Projector,
        })
        .await
        .expect("enqueue")
        .expect("queued");
    store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim")
        .expect("first claim");
    store
        .mark_egress_failed(id, "HTTP 502", Some(Utc::now() - Duration::seconds(1)))
        .await
        .expect("fail");
    let retry = store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim")
        .expect("retry claim");
    assert_eq!(retry.attempts, 2);

    store
        .defer_egress(id, Utc::now() + Duration::seconds(30))
        .await
        .expect("defer");
    assert!(
        store
            .claim_next_due_egress(Utc::now(), 300)
            .await
            .expect("claim")
            .is_none(),
        "a deferred delivery is not due"
    );
    let later = store
        .claim_next_due_egress(Utc::now() + Duration::seconds(31), 300)
        .await
        .expect("claim")
        .expect("due after the deferral");
    assert_eq!(later.attempts, 2, "the deferred claim did not count");

    store
        .mark_egress_failed(id, "gave up", None)
        .await
        .expect("dead");
    store
        .defer_egress(id, Utc::now() - Duration::seconds(1))
        .await
        .expect("defer dead");
    assert_eq!(store.count_dead_egress().await.expect("count"), 1);
}

async fn run_suite(store: &dyn Store) {
    webhook_suite(store).await;
    automation_suite(store).await;
    mail_suite(store).await;
    egress_suite(store).await;
}

#[tokio::test]
async fn a_deferred_delivery_keeps_its_attempts_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_deferred_delivery_keeps_its_attempts_postgres() {
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
