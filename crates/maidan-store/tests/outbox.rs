//! Postgres outbox integration tests.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{postgres::outbox, prelude::*, run_postgres_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn postgres_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, sqlx::PgPool)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping outbox tests: docker unavailable ({err})");
            return None;
        }
    };

    let host = container.get_host().await.ok()?;
    let port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .ok()?;

    run_postgres_migrations(&pool).await.ok()?;
    Some((container, pool))
}

fn workspace_created_event(name: &str) -> Event {
    Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: name.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    }
}

#[tokio::test]
async fn append_enqueues_unpublished_outbox_row() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let event = workspace_created_event("outbox-ws");
    let stored = store.append_event(&event).await.unwrap();
    assert!(outbox::count_pending(&pool).await.unwrap() >= 1);

    let pending = outbox::list_pending(&pool, 8).await.unwrap();
    assert!(pending.iter().any(|row| row.log_id == stored.id));
    assert_eq!(pending[0].attempts, 0);
}

#[tokio::test]
async fn record_attempt_increments_attempts_while_row_stays_pending() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let stored = store
        .append_event(&workspace_created_event("attempts-ws"))
        .await
        .unwrap();
    let pending = outbox::list_pending(&pool, 1).await.unwrap();
    let row = pending
        .into_iter()
        .find(|r| r.log_id == stored.id)
        .expect("pending row");

    assert_eq!(outbox::record_attempt(&pool, row.id).await.unwrap(), 1);
    assert_eq!(outbox::record_attempt(&pool, row.id).await.unwrap(), 2);

    let again = outbox::list_pending(&pool, 8).await.unwrap();
    let updated = again
        .into_iter()
        .find(|r| r.id == row.id)
        .expect("still pending");
    assert_eq!(updated.attempts, 2);
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 1);
}

#[tokio::test]
async fn mark_published_clears_pending_and_rejects_unknown_id() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let stored = store
        .append_event(&workspace_created_event("published-ws"))
        .await
        .unwrap();
    let pending = outbox::list_pending(&pool, 1).await.unwrap();
    let row = pending
        .into_iter()
        .find(|r| r.log_id == stored.id)
        .expect("pending row");

    outbox::mark_published(&pool, row.id).await.unwrap();
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);

    let err = outbox::mark_published(&pool, row.id).await.unwrap_err();
    assert!(matches!(err, maidan_store::StoreError::NotFound));

    let err = outbox::mark_published(&pool, 9_999_999).await.unwrap_err();
    assert!(matches!(err, maidan_store::StoreError::NotFound));
}

#[tokio::test]
async fn list_pending_joins_the_event_payload() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let stored = store
        .append_event(&workspace_created_event("payload-ws"))
        .await
        .unwrap();
    let pending = outbox::list_pending(&pool, 8).await.unwrap();
    let row = pending
        .into_iter()
        .find(|r| r.log_id == stored.id)
        .expect("pending row");
    // H4: the payload rides the pending list, so the relay never re-fetches it.
    assert!(row.payload.is_object());
    let event: Event = serde_json::from_value(row.payload).unwrap();
    assert!(matches!(event, Event::WorkspaceCreated { .. }));
}

#[tokio::test]
async fn mark_published_batch_clears_all_pending_and_is_idempotent() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    for name in ["batch-a", "batch-b", "batch-c"] {
        store
            .append_event(&workspace_created_event(name))
            .await
            .unwrap();
    }
    let ids: Vec<i64> = outbox::list_pending(&pool, 8)
        .await
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids.len(), 3);

    outbox::mark_published_batch(&pool, &ids).await.unwrap();
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);

    // Idempotent: already-published rows are skipped, no error; empty is a no-op.
    outbox::mark_published_batch(&pool, &ids).await.unwrap();
    outbox::mark_published_batch(&pool, &[]).await.unwrap();
}

#[tokio::test]
async fn list_pending_orders_by_id_and_respects_limit() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let first = store
        .append_event(&workspace_created_event("order-a"))
        .await
        .unwrap();
    let second = store
        .append_event(&workspace_created_event("order-b"))
        .await
        .unwrap();

    let one = outbox::list_pending(&pool, 1).await.unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].log_id, first.id);

    let two = outbox::list_pending(&pool, 2).await.unwrap();
    assert_eq!(two.len(), 2);
    assert!(two[0].id < two[1].id);
    assert_eq!(two[1].log_id, second.id);
}

#[tokio::test]
async fn multiple_appends_enqueue_one_outbox_row_per_event() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    store
        .append_event(&workspace_created_event("multi-a"))
        .await
        .unwrap();
    store
        .append_event(&workspace_created_event("multi-b"))
        .await
        .unwrap();
    store
        .append_event(&workspace_created_event("multi-c"))
        .await
        .unwrap();

    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 3);
}

#[tokio::test]
async fn quarantined_rows_are_excluded_from_pending_list_and_count() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    store
        .append_event(&workspace_created_event("q-ws"))
        .await
        .unwrap();
    let pending = outbox::list_pending(&pool, 1).await.unwrap();
    outbox::quarantine(&pool, pending[0].id).await.unwrap();

    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);
    assert_eq!(outbox::count_quarantined(&pool).await.unwrap(), 1);
    assert!(outbox::list_pending(&pool, 8).await.unwrap().is_empty());
}

/// Two relays must not claim the same row.
///
/// The relay is spawned in **every** replica and `validate_startup` refuses to
/// disable it in production, so the old unlocked `list_pending` had every
/// replica relay every row. Downstream that is not a benign duplicate:
/// `maidan_webhook_deliveries` has no unique on `(subscription_id, log_id)`, so
/// each tenant endpoint got N POSTs, and `fsm_hook_worker` re-fired every hook
/// through `dispatch_mcp_tool` with `AuthContext::bypass()`.
#[tokio::test]
async fn concurrent_relays_claim_disjoint_outbox_rows() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };
    let store = PostgresStore::new(pool.clone());

    // Drain anything a sibling test left behind so the counts below are ours.
    loop {
        let drained = outbox::claim_pending(&pool, 256, 0).await.unwrap();
        if drained.is_empty() {
            break;
        }
        let ids: Vec<i64> = drained.iter().map(|r| r.id).collect();
        outbox::mark_published_batch(&pool, &ids).await.unwrap();
    }

    let mut expected = Vec::new();
    for i in 0..6 {
        let stored = store
            .append_event(&workspace_created_event(&format!("claim-ws-{i}")))
            .await
            .unwrap();
        expected.push(stored.id);
    }

    // Prove the hazard is real before proving the fix: the unlocked read that
    // the relay used to call hands BOTH callers the same rows.
    let listed_a = outbox::list_pending(&pool, 6).await.unwrap();
    let listed_b = outbox::list_pending(&pool, 6).await.unwrap();
    let la: std::collections::HashSet<i64> = listed_a.iter().map(|r| r.log_id).collect();
    let lb: std::collections::HashSet<i64> = listed_b.iter().map(|r| r.log_id).collect();
    assert!(
        !la.is_disjoint(&lb) && !la.is_empty(),
        "list_pending must still overlap — otherwise this test proves nothing about the claim"
    );

    // Two relays claim concurrently, exactly as two replicas would.
    let a = outbox::claim_pending(&pool, 6, 60);
    let b = outbox::claim_pending(&pool, 6, 60);
    let (claimed_a, claimed_b) = tokio::join!(a, b);
    let claimed_a = claimed_a.unwrap();
    let claimed_b = claimed_b.unwrap();

    let ids_a: std::collections::HashSet<i64> = claimed_a.iter().map(|r| r.log_id).collect();
    let ids_b: std::collections::HashSet<i64> = claimed_b.iter().map(|r| r.log_id).collect();
    let overlap: Vec<_> = ids_a.intersection(&ids_b).collect();
    assert!(
        overlap.is_empty(),
        "two relays claimed the same rows: {overlap:?} — every event would relay twice"
    );

    // Between them they still see every row: a claim must not drop work.
    for id in &expected {
        assert!(
            ids_a.contains(id) || ids_b.contains(id),
            "log_id {id} was claimed by neither relay"
        );
    }

    // A claimed row is not claimable again while the lease holds...
    assert!(
        outbox::claim_pending(&pool, 6, 60)
            .await
            .unwrap()
            .is_empty(),
        "a live claim must exclude the row"
    );
    // ...but a failure releases it, so a retry is not stuck behind the lease.
    let first = claimed_a.first().or_else(|| claimed_b.first()).unwrap();
    outbox::record_attempt(&pool, first.id).await.unwrap();
    let reclaimed = outbox::claim_pending(&pool, 6, 60).await.unwrap();
    assert_eq!(
        reclaimed.len(),
        1,
        "record_attempt should release the claim for retry"
    );
    assert_eq!(reclaimed[0].id, first.id);

    // And an expired lease is reclaimable, so a crashed relay strands nothing.
    let stale = outbox::claim_pending(&pool, 6, 0).await.unwrap();
    assert!(
        !stale.is_empty(),
        "an expired claim must be reclaimable — otherwise a crashed relay strands the row"
    );
}
