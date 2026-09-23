//! The attribution mechanism, on both backends: events and audit rows record
//! the principal in scope, record nothing outside one, and the event chain
//! still verifies with attribution inside the hashed payload.

use maidan_store::attribution::with_attribution;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Attribution, DelegationGrantId, MemberKind, NewAuditEvent, NewChannel, NewMember, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
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
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for handle in ["delegate", "subject", "bystander"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let (delegate, subject, bystander) = (ids[0], ids[1], ids[2]);
    let delegated = Attribution {
        actor_id: delegate,
        subject_id: subject,
        grant_id: Some(DelegationGrantId(uuid::Uuid::new_v4())),
    };
    let channel = |name: &'static str| NewChannel {
        workspace_id: ws.id,
        name: name.into(),
        topic: None,
        private: false,
    };

    // Outside any request, nothing is claimed: this is how background work reads.
    let (_, unscoped) = store
        .create_channel_with_event(channel("unscoped"))
        .await
        .unwrap();
    assert_eq!(unscoped.attribution(), None);

    // Inside a delegated scope the event names delegate, subject and grant.
    let (_, scoped) = with_attribution(
        Some(delegated),
        store.create_channel_with_event(channel("scoped")),
    )
    .await
    .unwrap();
    assert_eq!(scoped.attribution(), Some(delegated));

    // Attribution sits inside the hashed payload and the chain still verifies,
    // on this backend's own hashing.
    assert!(store.verify_event_chain(ws.id).await.unwrap().ok);

    let audit = |actor| NewAuditEvent {
        actor_id: actor,
        action: "test.action".into(),
        target_kind: Some("workspace".into()),
        target_id: Some(ws.id.0),
        metadata: serde_json::json!({}),
    };

    // Outside a scope the caller names the actor, who acted for itself.
    let row = store.append_audit(audit(Some(bystander))).await.unwrap();
    assert_eq!(row.actor_id, Some(bystander));
    assert_eq!(row.subject_id, Some(bystander));
    assert_eq!(row.grant_id, None);

    // Inside one the scope wins, even over a caller naming the wrong actor —
    // the bug this replaces was call sites recording the subject as actor.
    let row = with_attribution(Some(delegated), store.append_audit(audit(Some(subject))))
        .await
        .unwrap();
    assert_eq!(row.actor_id, Some(delegate));
    assert_eq!(row.subject_id, Some(subject));
    assert_eq!(row.grant_id, delegated.grant_id);

    // And the columns come back through the list reads, not just the insert.
    let listed = store
        .list_audit_for_workspace(ws.id, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == row.id)
        .expect("the delegated row is listed");
    assert_eq!(listed.subject_id, Some(subject));
    assert_eq!(listed.grant_id, delegated.grant_id);
}

#[tokio::test]
async fn attribution_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn attribution_postgres() {
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
        Ok(container) => container,
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
    run_suite(&PostgresStore::new(pool)).await;
}
