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

    // First reservation wins; a second one sees it in flight.
    let k1 = new(alice.id, "k1", "fp-a", Duration::minutes(5));
    assert_eq!(
        store.reserve_idempotency_key(&k1).await.expect("reserve"),
        IdempotencyReservation::Reserved
    );
    assert_eq!(
        store.reserve_idempotency_key(&k1).await.expect("again"),
        IdempotencyReservation::InFlight {
            fingerprint: "fp-a".into()
        }
    );

    // The same key under another actor is independent.
    assert_eq!(
        store
            .reserve_idempotency_key(&new(bob.id, "k1", "fp-b", Duration::minutes(5)))
            .await
            .expect("bob"),
        IdempotencyReservation::Reserved
    );

    // Completing stores the response; a retry gets it back.
    let response = StoredResponse {
        status: 201,
        content_type: Some("application/json".into()),
        body: br#"{"id":"x"}"#.to_vec(),
    };
    store
        .complete_idempotency_key(ws.id, alice.id, "k1", &response)
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
        .release_idempotency_key(ws.id, bob.id, "k1")
        .await
        .expect("release");
    assert_eq!(
        store
            .reserve_idempotency_key(&new(bob.id, "k1", "fp-b2", Duration::minutes(5)))
            .await
            .expect("after release"),
        IdempotencyReservation::Reserved
    );

    // A lapsed lock that never completed is taken over by the next request.
    let lapsed = new(alice.id, "k2", "fp-old", -Duration::seconds(1));
    assert_eq!(
        store
            .reserve_idempotency_key(&lapsed)
            .await
            .expect("lapsed"),
        IdempotencyReservation::Reserved
    );
    let retry = new(alice.id, "k2", "fp-new", Duration::minutes(5));
    assert_eq!(
        store
            .reserve_idempotency_key(&retry)
            .await
            .expect("takeover"),
        IdempotencyReservation::Reserved
    );
    assert_eq!(
        store.reserve_idempotency_key(&retry).await.expect("held"),
        IdempotencyReservation::InFlight {
            fingerprint: "fp-new".into()
        }
    );

    // An expired completed key is pruned and the key is free again.
    let mut old = new(alice.id, "k3", "fp-3", -Duration::hours(2));
    old.expires_at = now - Duration::hours(1);
    assert_eq!(
        store.reserve_idempotency_key(&old).await.expect("old"),
        IdempotencyReservation::Reserved
    );
    store
        .complete_idempotency_key(ws.id, alice.id, "k3", &response)
        .await
        .expect("complete old");
    assert_eq!(
        store
            .reserve_idempotency_key(&new(alice.id, "k3", "fp-3b", Duration::minutes(5)))
            .await
            .expect("expired"),
        IdempotencyReservation::Reserved
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
