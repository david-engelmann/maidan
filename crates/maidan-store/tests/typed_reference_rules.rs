//! Write-time rules on `add_reference`: no self-link, both ends must exist in
//! the same workspace, and `depends` / `supersedes` / `duplicates` stay
//! acyclic. Free-form `Other` relations and mixed thread/message pairs stay
//! allowed. Identical edges stay a 409.

use std::fmt::Debug;

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Member, MemberKind, NewChannel, NewMember, NewMessage, NewReference, NewThread, NewWorkspace,
    RefSide, RelationKind, Thread, WorkspaceId,
};

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

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> Member {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member")
}

async fn thread_in(store: &dyn Store, ws: WorkspaceId, name: &str) -> Thread {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: name.into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some(name.into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
}

fn edge(
    src_kind: RefSide,
    src: uuid::Uuid,
    dst_kind: RefSide,
    dst: uuid::Uuid,
    relation: RelationKind,
) -> NewReference {
    NewReference {
        src_kind,
        src_id: src,
        dst_kind,
        dst_id: dst,
        relation,
    }
}

fn invalid_input<T: Debug>(what: &str, result: Result<T, StoreError>, needle: &str) {
    match result {
        Err(StoreError::InvalidInput(msg)) if msg.contains(needle) => {}
        other => panic!("{what}: expected InvalidInput containing {needle:?}, got {other:?}"),
    }
}

async fn run_suite(store: &dyn Store) {
    let ws_a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .expect("ws a");
    let ws_b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .expect("ws b");
    let alice = member(store, ws_a.id, "alice").await;
    let bob = member(store, ws_b.id, "bob").await;
    let t_a1 = thread_in(store, ws_a.id, "a1").await;
    let t_a2 = thread_in(store, ws_a.id, "a2").await;
    let t_a3 = thread_in(store, ws_a.id, "a3").await;
    let t_b = thread_in(store, ws_b.id, "b1").await;
    let msg_a = store
        .post_message(NewMessage {
            thread_id: t_a1.id,
            author_id: alice.id,
            body: "in a".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("msg a");
    let msg_b = store
        .post_message(NewMessage {
            thread_id: t_b.id,
            author_id: bob.id,
            body: "in b".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("msg b");

    invalid_input(
        "self-link thread",
        store
            .add_reference(edge(
                RefSide::Thread,
                t_a1.id.0,
                RefSide::Thread,
                t_a1.id.0,
                RelationKind::Supports,
            ))
            .await,
        "cannot point at itself",
    );
    invalid_input(
        "self-link message",
        store
            .add_reference(edge(
                RefSide::Message,
                msg_a.id.0,
                RefSide::Message,
                msg_a.id.0,
                RelationKind::from_wire("twin_of"),
            ))
            .await,
        "cannot point at itself",
    );
    invalid_input(
        "self-link via event",
        store
            .add_reference_with_event(edge(
                RefSide::Thread,
                t_a1.id.0,
                RefSide::Thread,
                t_a1.id.0,
                RelationKind::Depends,
            ))
            .await,
        "cannot point at itself",
    );

    let missing = uuid::Uuid::now_v7();
    invalid_input(
        "missing source",
        store
            .add_reference(edge(
                RefSide::Thread,
                missing,
                RefSide::Thread,
                t_a1.id.0,
                RelationKind::Supports,
            ))
            .await,
        "source does not exist",
    );
    invalid_input(
        "missing target",
        store
            .add_reference(edge(
                RefSide::Message,
                msg_a.id.0,
                RefSide::Message,
                missing,
                RelationKind::Refutes,
            ))
            .await,
        "target does not exist",
    );

    invalid_input(
        "cross-workspace thread",
        store
            .add_reference(edge(
                RefSide::Thread,
                t_a1.id.0,
                RefSide::Thread,
                t_b.id.0,
                RelationKind::from_wire("twin_of"),
            ))
            .await,
        "cannot cross workspaces",
    );
    invalid_input(
        "cross-workspace message to thread",
        store
            .add_reference(edge(
                RefSide::Message,
                msg_a.id.0,
                RefSide::Thread,
                t_b.id.0,
                RelationKind::Supports,
            ))
            .await,
        "cannot cross workspaces",
    );
    invalid_input(
        "cross-workspace via event",
        store
            .add_reference_with_event(edge(
                RefSide::Message,
                msg_b.id.0,
                RefSide::Message,
                msg_a.id.0,
                RelationKind::Depends,
            ))
            .await,
        "cannot cross workspaces",
    );

    store
        .add_reference(edge(
            RefSide::Thread,
            t_a1.id.0,
            RefSide::Thread,
            t_a2.id.0,
            RelationKind::Depends,
        ))
        .await
        .expect("a1 depends a2");
    invalid_input(
        "depends two-cycle",
        store
            .add_reference(edge(
                RefSide::Thread,
                t_a2.id.0,
                RefSide::Thread,
                t_a1.id.0,
                RelationKind::Depends,
            ))
            .await,
        "would create a cycle",
    );
    store
        .add_reference(edge(
            RefSide::Thread,
            t_a2.id.0,
            RefSide::Thread,
            t_a3.id.0,
            RelationKind::Depends,
        ))
        .await
        .expect("a2 depends a3");
    invalid_input(
        "depends three-cycle",
        store
            .add_reference(edge(
                RefSide::Thread,
                t_a3.id.0,
                RefSide::Thread,
                t_a1.id.0,
                RelationKind::Depends,
            ))
            .await,
        "would create a cycle",
    );

    store
        .add_reference(edge(
            RefSide::Thread,
            t_a1.id.0,
            RefSide::Thread,
            t_a2.id.0,
            RelationKind::Supersedes,
        ))
        .await
        .expect("a1 supersedes a2");
    invalid_input(
        "supersedes cycle",
        store
            .add_reference(edge(
                RefSide::Thread,
                t_a2.id.0,
                RefSide::Thread,
                t_a1.id.0,
                RelationKind::Supersedes,
            ))
            .await,
        "would create a cycle",
    );
    store
        .add_reference(edge(
            RefSide::Thread,
            t_a1.id.0,
            RefSide::Thread,
            t_a2.id.0,
            RelationKind::Duplicates,
        ))
        .await
        .expect("a1 duplicates a2");
    invalid_input(
        "duplicates cycle",
        store
            .add_reference(edge(
                RefSide::Thread,
                t_a2.id.0,
                RefSide::Thread,
                t_a1.id.0,
                RelationKind::Duplicates,
            ))
            .await,
        "would create a cycle",
    );

    store
        .add_reference(edge(
            RefSide::Message,
            msg_a.id.0,
            RefSide::Thread,
            t_a3.id.0,
            RelationKind::Depends,
        ))
        .await
        .expect("message depends thread");
    invalid_input(
        "mixed-kind depends cycle",
        store
            .add_reference(edge(
                RefSide::Thread,
                t_a3.id.0,
                RefSide::Message,
                msg_a.id.0,
                RelationKind::Depends,
            ))
            .await,
        "would create a cycle",
    );

    // Supports and Other may form cycles; mixed kinds stay legal.
    let msg_a2 = store
        .post_message(NewMessage {
            thread_id: t_a2.id,
            author_id: alice.id,
            body: "also in a".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("msg a2");
    store
        .add_reference(edge(
            RefSide::Message,
            msg_a.id.0,
            RefSide::Message,
            msg_a2.id.0,
            RelationKind::Supports,
        ))
        .await
        .expect("supports forward");
    store
        .add_reference(edge(
            RefSide::Message,
            msg_a2.id.0,
            RefSide::Message,
            msg_a.id.0,
            RelationKind::Supports,
        ))
        .await
        .expect("supports cycle is allowed");
    store
        .add_reference(edge(
            RefSide::Thread,
            t_a1.id.0,
            RefSide::Thread,
            t_a3.id.0,
            RelationKind::from_wire("twin_of"),
        ))
        .await
        .expect("other forward");
    store
        .add_reference(edge(
            RefSide::Thread,
            t_a3.id.0,
            RefSide::Thread,
            t_a1.id.0,
            RelationKind::from_wire("twin_of"),
        ))
        .await
        .expect("other cycle is allowed");
    store
        .add_reference(edge(
            RefSide::Message,
            msg_b.id.0,
            RefSide::Thread,
            t_b.id.0,
            RelationKind::Grounds,
        ))
        .await
        .expect("mixed kinds in one workspace");

    let once = edge(
        RefSide::Message,
        msg_a.id.0,
        RefSide::Thread,
        t_a1.id.0,
        RelationKind::Supports,
    );
    store.add_reference(once.clone()).await.expect("first edge");
    let again = store.add_reference(once).await;
    assert!(
        matches!(again, Err(StoreError::Conflict(ref m)) if m.contains("already exists")),
        "identical edge stays a conflict, got {again:?}"
    );
}

#[tokio::test]
async fn add_reference_enforces_the_rule_set_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn add_reference_enforces_the_rule_set_postgres() {
    use std::time::Duration;

    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers::ImageExt;
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
