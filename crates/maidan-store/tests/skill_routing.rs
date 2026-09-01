//! Skill routing (Cluster 231): a task's required skills gate `claim_next` — a
//! member only claims a task whose required skills it holds. Plus the
//! `thread_required_skills` CRUD. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
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
            name: "route".into(),
        })
        .await
        .expect("ws");
    let mk_member = |handle: &str| NewMember {
        workspace_id: ws.id,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Agent,
    };
    // alice can code-review + rust; bob only rust.
    let alice = store
        .create_member(mk_member("alice"))
        .await
        .expect("alice");
    let bob = store.create_member(mk_member("bob")).await.expect("bob");
    store.add_member_skill(alice.id, "rust").await.expect("a1");
    store
        .add_member_skill(alice.id, "code-review")
        .await
        .expect("a2");
    store.add_member_skill(bob.id, "rust").await.expect("b1");

    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };
    // `review` (created first, so oldest) requires code-review; `open` has no
    // requirement.
    let review = store.create_thread(mk("review")).await.expect("review");
    let open = store.create_thread(mk("open")).await.expect("open");
    store
        .add_thread_required_skill(review.id, "code-review")
        .await
        .expect("req");
    // Idempotent + empty-reject.
    store
        .add_thread_required_skill(review.id, "code-review")
        .await
        .expect("req idem");
    assert!(matches!(
        store.add_thread_required_skill(review.id, "  ").await,
        Err(maidan_store::StoreError::InvalidInput(_))
    ));
    let reqs = store
        .list_thread_required_skills(review.id)
        .await
        .expect("list req");
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].skill, "code-review");

    // bob can't do `review` (lacks code-review), so despite it being older he
    // claims `open`.
    let b = store
        .claim_next_thread(channel.id, bob.id, None)
        .await
        .expect("bob claim")
        .expect("bob gets work");
    assert_eq!(b.id, open.id, "bob skips the review task he can't do");

    // alice has code-review, so she claims `review`.
    let a = store
        .claim_next_thread(channel.id, alice.id, None)
        .await
        .expect("alice claim")
        .expect("alice gets work");
    assert_eq!(a.id, review.id);

    // Nothing claimable now.
    assert!(store
        .claim_next_thread(channel.id, alice.id, None)
        .await
        .expect("empty")
        .is_none());

    // A task requiring a skill nobody has is unclaimable until the requirement
    // is dropped.
    let hard = store.create_thread(mk("hard")).await.expect("hard");
    store
        .add_thread_required_skill(hard.id, "quantum")
        .await
        .expect("req quantum");
    assert!(store
        .claim_next_thread(channel.id, alice.id, None)
        .await
        .expect("blocked")
        .is_none());
    assert!(store
        .remove_thread_required_skill(hard.id, "quantum")
        .await
        .expect("drop req"));
    let a2 = store
        .claim_next_thread(channel.id, alice.id, None)
        .await
        .expect("after drop")
        .expect("now claimable");
    assert_eq!(a2.id, hard.id);
}

#[tokio::test]
async fn skill_routing_gates_claim_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn skill_routing_gates_claim_postgres() {
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
