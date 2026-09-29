//! Idempotency keys: reserve / complete / release and lapsed-lock takeover,
//! the same on both backends.

use chrono::{Duration, Utc};
use maidan_store::{
    prelude::*, run_sqlite_migrations, IdempotencyReservation, NewIdempotencyKey, StoredResponse,
};
use maidan_types::{MemberKind, NewMember, NewWorkspace};
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

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "idem".into(),
        })
        .await
        .expect("ws");
    let member = |handle: &str| NewMember {
        workspace_id: ws.id,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Agent,
    };
    let alice = store.create_member(member("alice")).await.expect("alice");
    let bob = store.create_member(member("bob")).await.expect("bob");
    let now = Utc::now();
    let new = |actor, key: &str, fp: &str, lock: Duration| NewIdempotencyKey {
        workspace_id: ws.id,
        actor_id: actor,
        key: key.into(),
        fingerprint: fp.into(),
        locked_until: now + lock,
        expires_at: now + Duration::hours(24),
    };

    let lease = |r: IdempotencyReservation| match r {
        IdempotencyReservation::Reserved { lease } => lease,
        other => panic!("expected Reserved, got {other:?}"),
    };

    // First reservation wins; a second one sees it in flight.
    let k1 = new(alice.id, "k1", "fp-a", Duration::minutes(5));
    let alice_k1 = lease(store.reserve_idempotency_key(&k1).await.expect("reserve"));
    assert_eq!(
        store.reserve_idempotency_key(&k1).await.expect("again"),
        IdempotencyReservation::InFlight {
            fingerprint: "fp-a".into()
        }
    );

    // The same key under another actor is independent.
    let bob_k1 = lease(
        store
            .reserve_idempotency_key(&new(bob.id, "k1", "fp-b", Duration::minutes(5)))
            .await
            .expect("bob"),
    );

    // Completing under the wrong lease does nothing; under the right one it
    // stores the response and a retry gets it back.
    let response = StoredResponse {
        status: 201,
        content_type: Some("application/json".into()),
        body: br#"{"id":"x"}"#.to_vec(),
    };
    store
        .complete_idempotency_key(ws.id, alice.id, "k1", &bob_k1, &response)
        .await
        .expect("complete, wrong lease");
    assert!(matches!(
        store
            .reserve_idempotency_key(&k1)
            .await
            .expect("still held"),
        IdempotencyReservation::InFlight { .. }
    ));
    store
        .complete_idempotency_key(ws.id, alice.id, "k1", &alice_k1, &response)
        .await
        .expect("complete");
    assert_eq!(
        store.reserve_idempotency_key(&k1).await.expect("replay"),
        IdempotencyReservation::Completed {
            fingerprint: "fp-a".into(),
            response: response.clone(),
        }
    );

    // Releasing lets a retry run again.
    store
        .release_idempotency_key(ws.id, bob.id, "k1", &bob_k1)
        .await
        .expect("release");
    lease(
        store
            .reserve_idempotency_key(&new(bob.id, "k1", "fp-b2", Duration::minutes(5)))
            .await
            .expect("after release"),
    );

    // A lapsed lock that never completed is taken over by a retry of the
    // same request, not by a different one; the old holder is fenced out.
    let old_lease = lease(
        store
            .reserve_idempotency_key(&new(alice.id, "k2", "fp-old", -Duration::seconds(1)))
            .await
            .expect("lapsed"),
    );
    assert_eq!(
        store
            .reserve_idempotency_key(&new(alice.id, "k2", "fp-other", Duration::minutes(5)))
            .await
            .expect("different request"),
        IdempotencyReservation::InFlight {
            fingerprint: "fp-old".into()
        }
    );
    let retry = new(alice.id, "k2", "fp-old", Duration::minutes(5));
    let new_lease = lease(
        store
            .reserve_idempotency_key(&retry)
            .await
            .expect("takeover"),
    );
    assert_ne!(old_lease, new_lease);
    store
        .release_idempotency_key(ws.id, alice.id, "k2", &old_lease)
        .await
        .expect("stale release");
    store
        .complete_idempotency_key(ws.id, alice.id, "k2", &old_lease, &response)
        .await
        .expect("stale complete");
    assert_eq!(
        store.reserve_idempotency_key(&retry).await.expect("held"),
        IdempotencyReservation::InFlight {
            fingerprint: "fp-old".into()
        }
    );

    // An expired completed key is gone: the next request under it runs.
    let mut old = new(alice.id, "k3", "fp-3", -Duration::hours(2));
    old.expires_at = now - Duration::hours(1);
    let l = lease(store.reserve_idempotency_key(&old).await.expect("old"));
    store
        .complete_idempotency_key(ws.id, alice.id, "k3", &l, &response)
        .await
        .expect("complete old");
    lease(
        store
            .reserve_idempotency_key(&new(alice.id, "k3", "fp-3b", Duration::minutes(5)))
            .await
            .expect("expired"),
    );
}

#[tokio::test]
async fn idempotency_keys_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn idempotency_keys_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
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
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
