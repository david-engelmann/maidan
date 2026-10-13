//! Managed Postgres (Neon, PlanetScale, RDS, Cloud SQL) requires TLS, and the
//! store reached it through a sqlx built without any TLS support: a
//! `sslmode=require` URL failed with "SQLx was built without TLS support"
//! before it reached the server. This pins that the TLS client is compiled in,
//! by asking a plain (non-TLS) Postgres for TLS: the refusal must come from the
//! server, not from a missing client.

use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn a_tls_url_reaches_the_server_instead_of_failing_for_want_of_a_tls_client() {
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

    // Without TLS the same server accepts the connection.
    let plain = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&plain)
        .await
        .expect("a plain connection works");

    let tls = format!("{plain}?sslmode=require");
    let err = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&tls)
        .await
        .expect_err("the test server offers no TLS, so a required TLS connection fails");
    let message = err.to_string();
    assert!(
        !message.contains("without TLS support"),
        "the TLS client must be compiled in: {message}"
    );
}
