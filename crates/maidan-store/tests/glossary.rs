//! Shared-glossary foundation (Cluster 321, fidelity arc): a workspace's canonical
//! `term -> definition` (+ aliases) — set (upsert) / get / list / delete. Both
//! backends. No routes/tools yet — a zero-blast-radius foundation.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewGlossaryTerm, NewMember, NewWorkspace};
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
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "glossary".into(),
        })
        .await
        .expect("ws");
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "editor".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");

    // Set two terms.
    let ttl = store
        .set_glossary_term(NewGlossaryTerm {
            workspace_id: ws.id,
            term: "TTL".into(),
            definition: "time to live".into(),
            aliases: vec!["time-to-live".into(), "expiry".into()],
            created_by: author.id,
        })
        .await
        .expect("set TTL");
    assert_eq!(ttl.term, "TTL");
    assert_eq!(ttl.aliases.len(), 2);
    assert_eq!(ttl.created_by, author.id);
    let created_at = ttl.created_at;

    store
        .set_glossary_term(NewGlossaryTerm {
            workspace_id: ws.id,
            term: "LSN".into(),
            definition: "log sequence number".into(),
            aliases: vec![],
            created_by: author.id,
        })
        .await
        .expect("set LSN");

    // get one.
    let got = store
        .get_glossary_term(ws.id, "TTL")
        .await
        .expect("get TTL")
        .expect("present");
    assert_eq!(got.definition, "time to live");
    assert_eq!(
        got.aliases,
        vec!["time-to-live".to_string(), "expiry".into()]
    );

    // Missing term -> None.
    assert!(store
        .get_glossary_term(ws.id, "nope")
        .await
        .expect("get missing")
        .is_none());

    // Re-set (upsert) overwrites the definition/aliases and keeps created_at.
    let updated = store
        .set_glossary_term(NewGlossaryTerm {
            workspace_id: ws.id,
            term: "TTL".into(),
            definition: "how long a cache entry stays valid".into(),
            aliases: vec!["expiry".into()],
            created_by: author.id,
        })
        .await
        .expect("re-set TTL");
    assert_eq!(updated.definition, "how long a cache entry stays valid");
    assert_eq!(updated.aliases, vec!["expiry".to_string()]);
    assert_eq!(
        updated.created_at, created_at,
        "created_at preserved on upsert"
    );

    // list is ordered by term and reflects the upsert (still 2 terms).
    let terms = store.list_glossary_terms(ws.id).await.expect("list");
    assert_eq!(terms.len(), 2, "upsert did not duplicate");
    assert_eq!(terms[0].term, "LSN");
    assert_eq!(terms[1].term, "TTL");
    assert_eq!(terms[1].definition, "how long a cache entry stays valid");

    // An empty term is rejected by the CHECK constraint.
    assert!(store
        .set_glossary_term(NewGlossaryTerm {
            workspace_id: ws.id,
            term: "".into(),
            definition: "x".into(),
            aliases: vec![],
            created_by: author.id,
        })
        .await
        .is_err());

    // delete is conditional.
    assert!(store
        .delete_glossary_term(ws.id, "TTL")
        .await
        .expect("delete TTL"));
    assert!(!store
        .delete_glossary_term(ws.id, "TTL")
        .await
        .expect("delete TTL again"));
    assert_eq!(
        store
            .list_glossary_terms(ws.id)
            .await
            .expect("list after delete")
            .len(),
        1
    );
}

#[tokio::test]
async fn glossary_set_get_list_delete_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn glossary_set_get_list_delete_postgres() {
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
            eprintln!("skipping: docker unavailable ({err})");
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
