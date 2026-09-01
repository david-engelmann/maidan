//! Automation delivery store (Cluster 68.0).

use maidan_store::{prelude::*, run_sqlite_migrations, AutomationDeliveryFilter};
use maidan_types::{
    AutomationSourceKind, NewAutomationDelivery, NewFsmHook, NewWorkspace, SlashHandlerKind,
    ThreadState,
};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn automation_delivery_pending_and_quarantine_round_trip() {
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
    let ws = store
        .create_workspace(NewWorkspace {
            name: "auto".into(),
        })
        .await
        .unwrap();
    let hook = store
        .create_fsm_hook(NewFsmHook {
            workspace_id: ws.id,
            label: None,
            from_state: Some(ThreadState::Open),
            to_state: Some(ThreadState::InReview),
            handler_kind: SlashHandlerKind::McpTool,
            handler_target: "post_message".into(),
            secret_ciphertext: String::new(),
        })
        .await
        .unwrap();
    let id = store
        .enqueue_automation_delivery(NewAutomationDelivery {
            workspace_id: ws.id,
            source_kind: AutomationSourceKind::FsmHook,
            source_id: hook.id.0,
            target_url: "http://127.0.0.1:9/nope".into(),
            header_name: "X-Maidan-Event".into(),
            header_value: "thread_state_changed".into(),
            payload: "{}".into(),
        })
        .await
        .unwrap();
    let pending = store.list_pending_automation_deliveries(10).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, id);
    store.quarantine_automation_delivery(id).await.unwrap();
    let dlq = store
        .list_automation_deliveries(ws.id, AutomationDeliveryFilter::DeadLetter, 10)
        .await
        .unwrap();
    assert_eq!(dlq.len(), 1);
    let replayed = store.replay_automation_delivery(id, ws.id).await.unwrap();
    assert!(replayed.quarantined_at.is_none());
    let pending2 = store.list_pending_automation_deliveries(10).await.unwrap();
    assert!(pending2.iter().any(|p| p.id == id));
}
