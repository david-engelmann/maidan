//! The scheduled chain verifier.
//!
//! Whole-chain verification was moved out of the search tap's restart path, on
//! the argument that verification-by-restart is an accident rather than a
//! control. That argument only holds if something schedules it — this is the
//! test that it does, and that it reports a real break rather than passing
//! vacuously.

use std::sync::Arc;

use maidan_server::chain_verify;
use maidan_store::{prelude::*, run_sqlite_migrations, SqliteStore};
use maidan_types::{MemberKind, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

async fn store() -> (Arc<dyn Store>, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    (Arc::new(SqliteStore::new(pool.clone())), pool)
}

#[tokio::test]
async fn the_verifier_passes_a_clean_log_and_names_a_broken_one() {
    let (store, pool) = store().await;

    // Two workspaces, both with events.
    let mut ids = Vec::new();
    for name in ["cv-a", "cv-b"] {
        let (ws, _) = store
            .create_workspace_with_event(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        store
            .create_member_with_event(NewMember {
                workspace_id: ws.id,
                handle: format!("{name}-m"),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        ids.push(ws.id);
    }

    // Both workspaces are discovered from the log, and both verify.
    let listed = store.workspace_ids_with_events().await.unwrap();
    assert_eq!(listed.len(), 2, "both workspaces have chains: {listed:?}");
    let (checked, broken) = chain_verify::sweep_once(&store).await;
    assert_eq!(checked, 2);
    assert_eq!(broken, 0, "a clean log must verify");

    // Tamper with one workspace's payload.
    let events = store.list_events_after(ids[0], 0, 10).await.unwrap();
    let victim = events.last().expect("an event to tamper");
    let mut payload = victim.payload.clone();
    payload["tampered"] = serde_json::json!(true);
    sqlx::query("UPDATE maidan_events SET payload = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(victim.id)
        .execute(&pool)
        .await
        .unwrap();

    // The sweep reports exactly one broken chain and still checks the other —
    // one tenant's tamper must not stop the instance learning about the rest.
    let (checked, broken) = chain_verify::sweep_once(&store).await;
    assert_eq!(checked, 2, "the sweep continues past a break");
    assert_eq!(broken, 1, "and reports exactly the broken one");
}

/// The verifier is opt-in. An unconfigured deployment must not silently acquire
/// a periodic full-log read on upgrade.
#[test]
fn the_verifier_is_off_unless_configured() {
    std::env::remove_var("MAIDAN_CHAIN_VERIFY_SECS");
    assert!(chain_verify::interval_from_env().is_none());

    std::env::set_var("MAIDAN_CHAIN_VERIFY_SECS", "0");
    assert!(
        chain_verify::interval_from_env().is_none(),
        "zero is off, not a busy loop"
    );

    std::env::set_var("MAIDAN_CHAIN_VERIFY_SECS", "not-a-number");
    assert!(chain_verify::interval_from_env().is_none());

    std::env::set_var("MAIDAN_CHAIN_VERIFY_SECS", "3600");
    assert_eq!(
        chain_verify::interval_from_env().map(|d| d.as_secs()),
        Some(3600)
    );
    std::env::remove_var("MAIDAN_CHAIN_VERIFY_SECS");
}
