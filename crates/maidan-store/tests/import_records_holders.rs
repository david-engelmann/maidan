//! Import records each imported thread's holder as having worked the thread,
//! as a claim would, so a holder who later lets go is still known to the
//! past-holder checks. Two imported workspaces, both backends.

use chrono::Utc;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;

fn member(ws: WorkspaceId, handle: &str) -> Member {
    let now = Utc::now();
    Member {
        id: MemberId(uuid::Uuid::new_v4()),
        workspace_id: ws,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Agent,
        created_at: now,
        updated_at: now,
        tombstoned_at: None,
    }
}

fn thread(ch: ChannelId, holder: Option<MemberId>) -> Thread {
    let now = Utc::now();
    Thread {
        id: ThreadId(uuid::Uuid::new_v4()),
        channel_id: ch,
        parent_thread_id: None,
        title: Some("t".into()),
        description: None,
        state: ThreadState::Open,
        assignee_id: holder,
        assignment_expires_at: None,
        claim_lease_id: holder.map(|_| ClaimLeaseId(uuid::Uuid::new_v4())),
        work_started_at: holder.map(|_| now),
        owner_id: None,
        created_at: now,
        updated_at: now,
        tombstoned_at: None,
        status: None,
        block: None,
        attributes: None,
        closed_without_review: false,
        last_progress_at: None,
        stalled: false,
        human_wait: None,
        notices: Vec::new(),
        plan_state: None,
        ci: Vec::new(),
        created_by: None,
    }
}

/// A workspace with two members, a thread held by the first, and an
/// unheld thread.
fn bundle(name: &str) -> WorkspaceImport {
    let now = Utc::now();
    let ws = WorkspaceId(uuid::Uuid::new_v4());
    let holder = member(ws, "holder");
    let other = member(ws, "other");
    let ch = ChannelId(uuid::Uuid::new_v4());
    WorkspaceImport {
        workspace: Workspace {
            id: ws,
            name: name.into(),
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        },
        threads: vec![thread(ch, Some(holder.id)), thread(ch, None)],
        members: vec![holder, other],
        channels: vec![Channel {
            id: ch,
            workspace_id: ws,
            name: "general".into(),
            topic: None,
            private: false,
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        }],
        channel_members: Vec::new(),
        messages: Vec::new(),
        message_edits: Vec::new(),
        pins: Vec::new(),
        references: Vec::new(),
    }
}

async fn suite(store: &dyn Store) {
    let a = bundle("a");
    let b = bundle("b");
    for w in [&a, &b] {
        store
            .import_workspace(w, WorkspaceOrigin::Operator)
            .await
            .unwrap();
    }
    for (w, them) in [(&a, &b), (&b, &a)] {
        let (held, unheld) = (w.threads[0].id, w.threads[1].id);
        let (holder, other) = (w.members[0].id, w.members[1].id);
        assert!(store.has_worked_thread(held, holder).await.unwrap());
        assert_eq!(store.list_thread_workers(held).await.unwrap(), vec![holder]);
        assert!(!store.has_worked_thread(held, other).await.unwrap());
        assert!(!store
            .has_worked_thread(held, them.members[0].id)
            .await
            .unwrap());
        assert!(store.list_thread_workers(unheld).await.unwrap().is_empty());
    }
}

async fn sqlite_store() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    SqliteStore::for_tests(pool)
}

/// A crafted bundle cannot leave a holder behind that the import did not
/// write in full: a holder that does not exist refuses the whole import, a
/// re-import adds no second row, and a later failure in the same import rolls
/// the holder back with the thread.
async fn crafted(store: &dyn Store) {
    let a = bundle("a");
    store
        .import_workspace(&a, WorkspaceOrigin::Operator)
        .await
        .unwrap();

    let mut ghost = bundle("ghost");
    ghost.threads[0].assignee_id = Some(MemberId(uuid::Uuid::new_v4()));
    assert!(store
        .import_workspace(&ghost, WorkspaceOrigin::Operator)
        .await
        .is_err());
    assert!(store
        .list_thread_workers(ghost.threads[0].id)
        .await
        .unwrap()
        .is_empty());

    assert!(store
        .import_workspace(&a, WorkspaceOrigin::Operator)
        .await
        .is_err());
    assert_eq!(
        store.list_thread_workers(a.threads[0].id).await.unwrap(),
        vec![a.members[0].id]
    );

    let mut torn = bundle("torn");
    torn.messages.push(Message {
        id: MessageId(uuid::Uuid::new_v4()),
        thread_id: ThreadId(uuid::Uuid::new_v4()),
        author_id: torn.members[0].id,
        body: "orphan".into(),
        metadata: serde_json::json!({}),
        content: None,
        posted_at: Utc::now(),
        edited_at: None,
        tombstoned_at: None,
    });
    assert!(store
        .import_workspace(&torn, WorkspaceOrigin::Operator)
        .await
        .is_err());
    assert!(store
        .list_thread_workers(torn.threads[0].id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_crafted_import_cannot_record_a_holder_it_did_not_import_in_full() {
    crafted(&sqlite_store().await).await;
}

/// Today a bundle whose thread names another workspace's member as holder
/// imports, and that member is recorded as having worked the thread.
#[tokio::test]
#[ignore = "fails until import refuses a holder from outside the bundle"]
async fn an_import_naming_another_workspaces_member_as_holder_is_refused() {
    let store = sqlite_store().await;
    let a = bundle("a");
    store
        .import_workspace(&a, WorkspaceOrigin::Operator)
        .await
        .unwrap();
    let mut foreign = bundle("foreign");
    foreign.threads[0].assignee_id = Some(a.members[0].id);
    assert!(store
        .import_workspace(&foreign, WorkspaceOrigin::Operator)
        .await
        .is_err());
    assert!(store
        .list_thread_workers(foreign.threads[0].id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn an_imported_threads_holder_is_recorded_as_having_worked_it_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn an_imported_threads_holder_is_recorded_as_having_worked_it_on_postgres() {
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
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    suite(&PostgresStore::for_tests(pool)).await;
}
