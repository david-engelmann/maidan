//! Capability registry foundation (Cluster 230): a member's declared skills —
//! add (idempotent) / remove (conditional) / list. Both backends. No routes yet.

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
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
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "skills".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");

    // Declare two skills; idempotent re-add is a no-op.
    store
        .add_member_skill(member.id, "rust")
        .await
        .expect("add rust");
    store
        .add_member_skill(member.id, "code-review")
        .await
        .expect("add code-review");
    store
        .add_member_skill(member.id, "rust")
        .await
        .expect("re-add rust");

    // An empty skill is rejected.
    assert!(matches!(
        store.add_member_skill(member.id, "   ").await,
        Err(maidan_store::StoreError::InvalidInput(_))
    ));

    // list is ordered by skill.
    let skills = store.list_member_skills(member.id).await.expect("list");
    assert_eq!(skills.len(), 2, "two distinct skills");
    assert_eq!(skills[0].skill, "code-review");
    assert_eq!(skills[1].skill, "rust");
    assert_eq!(skills[0].member_id, member.id);

    // remove is conditional.
    assert!(store
        .remove_member_skill(member.id, "rust")
        .await
        .expect("remove rust"));
    assert!(!store
        .remove_member_skill(member.id, "rust")
        .await
        .expect("remove rust again"));
    assert_eq!(
        store
            .list_member_skills(member.id)
            .await
            .expect("list after remove")
            .len(),
        1
    );
}

#[tokio::test]
async fn member_skills_add_list_remove_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn member_skills_add_list_remove_postgres() {
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
