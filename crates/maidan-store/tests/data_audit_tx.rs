//! Destroying, replacing and preserving workspace data (D-A, 413.3b), on both
//! backends.
//!
//! - A legal hold binds the store itself: purge, erase, message purge and a
//!   replacing import are refused inside their own transaction, whoever calls.
//! - Each audited change commits with its record: with audit inserts broken,
//!   none of them happens.
//! - A replacing import is one transaction: if the import fails, the workspace
//!   it would have replaced is still there.
//! - An audit row keeps naming its actor after the actor's workspace is erased.

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    Member, MemberId, MemberKind, MessageId, NewAuditEvent, NewChannel, NewMember, NewMessage,
    NewThread, NewWorkspace, WorkspaceId, WorkspaceImport,
};
use sqlx::sqlite::SqlitePoolOptions;

fn event(action: &str, actor: Option<MemberId>) -> NewAuditEvent {
    NewAuditEvent {
        actor_id: actor,
        action: action.into(),
        target_kind: None,
        target_id: None,
        metadata: serde_json::json!({}),
    }
}

fn audit_for<T>(action: &'static str, actor: Option<MemberId>) -> maidan_store::AuditFor<T> {
    Box::new(move |_| event(action, actor))
}

struct Seeded {
    workspace: WorkspaceId,
    member: Member,
    tombstoned: MessageId,
}

/// A workspace with one member, one live message and one tombstoned message.
async fn seed(store: &dyn Store, name: &str) -> Seeded {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-m"),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let post = |body: &str| NewMessage {
        thread_id: thread.id,
        author_id: member.id,
        body: body.into(),
        metadata: serde_json::json!({}),
        content: None,
    };
    store.post_message(post("live")).await.unwrap();
    let tombstoned = store.post_message(post("gone")).await.unwrap();
    store.tombstone_message(tombstoned.id).await.unwrap();
    Seeded {
        workspace: ws.id,
        member,
        tombstoned: tombstoned.id,
    }
}

fn is_hold_refusal(result: Result<impl std::fmt::Debug, StoreError>) -> bool {
    matches!(result, Err(StoreError::Conflict(ref m)) if m.contains("legal hold"))
}

/// A bundle naming `workspace` with the same member twice, so the import fails
/// on its second insert.
fn failing_import(store_ws: maidan_types::Workspace, member: &Member) -> WorkspaceImport {
    WorkspaceImport {
        workspace: store_ws,
        members: vec![member.clone(), member.clone()],
        channels: Vec::new(),
        channel_members: Vec::new(),
        threads: Vec::new(),
        messages: Vec::new(),
        message_edits: Vec::new(),
        pins: Vec::new(),
        references: Vec::new(),
    }
}

async fn run_suite<F, Fut>(store: &dyn Store, break_audit: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    // A hold binds every caller of the store, not only the handlers.
    let held = seed(store, "held").await;
    let hold = store
        .place_legal_hold_audited(
            held.workspace,
            "litigation",
            Some(held.member.id),
            audit_for("hold.place", Some(held.member.id)),
        )
        .await
        .unwrap();
    assert!(is_hold_refusal(
        store.purge_workspace_messages(held.workspace).await
    ));
    assert!(is_hold_refusal(store.erase_workspace(held.workspace).await));
    assert!(is_hold_refusal(store.purge_message(held.tombstoned).await));
    let bundle = WorkspaceImport {
        members: vec![held.member.clone()],
        ..failing_import(
            store.get_workspace(held.workspace).await.unwrap(),
            &held.member,
        )
    };
    assert!(is_hold_refusal(
        store
            .import_workspace_audited(&bundle, true, event("import", None))
            .await
    ));
    assert!(store.get_message(held.tombstoned).await.is_ok());
    assert!(store.get_workspace(held.workspace).await.is_ok());

    // A replacing import that fails leaves the workspace it would replace.
    let target = seed(store, "target").await;
    let bundle = failing_import(
        store.get_workspace(target.workspace).await.unwrap(),
        &target.member,
    );
    assert!(store
        .import_workspace_audited(&bundle, true, event("import", None))
        .await
        .is_err());
    assert!(
        store.get_workspace(target.workspace).await.is_ok(),
        "a failed import must not have erased the workspace"
    );
    assert!(store.get_message(target.tombstoned).await.is_ok());

    // An erased workspace's members stay named in the record.
    let erased = seed(store, "erased").await;
    let actor = erased.member.id;
    store
        .append_audit(event("before.erase", Some(actor)))
        .await
        .unwrap();
    store
        .erase_workspace_audited(erased.workspace, audit_for("erase", Some(actor)))
        .await
        .unwrap();
    let audit = store.list_audit(100).await.unwrap();
    for action in ["before.erase", "erase"] {
        let row = audit.iter().find(|row| row.action == action).unwrap();
        assert_eq!(row.actor_id, Some(actor), "{action} lost its actor");
    }

    // Lifting nothing records nothing.
    let unheld = seed(store, "unheld").await;
    assert!(!store
        .lift_legal_hold_audited(
            unheld.workspace,
            maidan_types::LegalHoldId::new(),
            event("lift.none", None),
        )
        .await
        .unwrap());
    assert!(!store
        .list_audit(100)
        .await
        .unwrap()
        .iter()
        .any(|row| row.action == "lift.none"));

    // With audit down, no audited change happens.
    let victim = seed(store, "victim").await;
    break_audit().await;

    assert!(store
        .purge_workspace_messages_audited(victim.workspace, audit_for("purge", None))
        .await
        .is_err());
    assert!(store.get_message(victim.tombstoned).await.is_ok());
    assert!(store
        .erase_workspace_audited(victim.workspace, audit_for("erase", None))
        .await
        .is_err());
    assert!(store.get_workspace(victim.workspace).await.is_ok());
    assert!(store
        .purge_message_audited(victim.tombstoned, event("purge", None))
        .await
        .is_err());
    assert!(store.get_message(victim.tombstoned).await.is_ok());
    assert!(store
        .place_legal_hold_audited(victim.workspace, "r", None, audit_for("place", None))
        .await
        .is_err());
    assert!(store
        .list_workspace_legal_holds(victim.workspace)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .lift_legal_hold_audited(held.workspace, hold.id, event("lift", None))
        .await
        .is_err());
    assert_eq!(
        store
            .list_workspace_legal_holds(held.workspace)
            .await
            .unwrap()
            .len(),
        1
    );
    let fresh = WorkspaceImport {
        workspace: maidan_types::Workspace {
            id: WorkspaceId::new(),
            ..store.get_workspace(victim.workspace).await.unwrap()
        },
        members: Vec::new(),
        ..failing_import(
            store.get_workspace(victim.workspace).await.unwrap(),
            &victim.member,
        )
    };
    let workspaces = store.count_workspaces().await.unwrap();
    assert!(store
        .import_workspace_audited(&fresh, false, event("import", None))
        .await
        .is_err());
    assert_eq!(store.count_workspaces().await.unwrap(), workspaces);
}

#[tokio::test]
async fn workspace_data_changes_need_their_record_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool.clone());
    run_suite(&store, || async {
        sqlx::query(
            "CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
             BEGIN SELECT RAISE(ABORT, 'audit down'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn workspace_data_changes_need_their_record_postgres() {
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
    let store = PostgresStore::new(pool.clone());
    run_suite(&store, || async {
        sqlx::query(
            "CREATE FUNCTION audit_down() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'audit down'; END $$ LANGUAGE plpgsql",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
             FOR EACH ROW EXECUTE FUNCTION audit_down()",
        )
        .execute(&pool)
        .await
        .unwrap();
    })
    .await;
}
