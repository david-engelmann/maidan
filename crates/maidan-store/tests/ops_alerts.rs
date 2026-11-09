//! Standing operator alerts: a key raised at a value posts once, until its
//! value changes or it is cleared, whichever process asks. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};

async fn sqlite() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
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
    let denied = "github-checks-denied:acme/app";
    assert!(
        store
            .raise_ops_alert(denied, "standing")
            .await
            .expect("raise"),
        "a condition's first raise posts"
    );
    assert!(
        !store
            .raise_ops_alert(denied, "standing")
            .await
            .expect("raise again"),
        "a standing condition does not post again"
    );

    assert!(store
        .raise_ops_alert("boot:C0123ABCDE", "v438.0.0")
        .await
        .expect("boot"));
    assert!(
        !store
            .raise_ops_alert("boot:C0123ABCDE", "v438.0.0")
            .await
            .expect("wake"),
        "the same build waking again is silent"
    );
    assert!(
        store
            .raise_ops_alert("boot:C0123ABCDE", "v439.0.0")
            .await
            .expect("deploy"),
        "a new build posts"
    );
    assert!(!store
        .raise_ops_alert("boot:C0123ABCDE", "v439.0.0")
        .await
        .expect("wake"));

    assert!(
        store.clear_ops_alert(denied).await.expect("clear"),
        "clearing a standing condition owes a resolved line"
    );
    assert!(
        !store.clear_ops_alert(denied).await.expect("clear again"),
        "it is resolved once"
    );
    assert!(
        store
            .raise_ops_alert(denied, "standing")
            .await
            .expect("return"),
        "a condition that clears and comes back posts again"
    );
    assert!(
        !store
            .clear_ops_alert("github-checks-denied:acme/never")
            .await
            .expect("clear unraised"),
        "a condition never raised owes nothing"
    );
    assert!(
        !store
            .raise_ops_alert("boot:C0123ABCDE", "v439.0.0")
            .await
            .expect("boot kept"),
        "clearing one key leaves the others standing"
    );
}

#[tokio::test]
async fn a_standing_ops_alert_posts_once_until_it_clears_or_changes_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn a_standing_ops_alert_posts_once_until_it_clears_or_changes_postgres() {
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
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
