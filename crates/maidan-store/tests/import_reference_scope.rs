//! A restore writes only into the workspace its bundle describes. Every
//! reference in the bundle (a member's or channel's workspace, a channel
//! membership, a thread's channel, parent, holder, owner or creator, a
//! message's thread or author, an edit, a pin, a cross-reference) must name
//! an entity the bundle itself defines, in its own workspace. A bundle that
//! reaches into another tenant is refused with a validation error and nothing
//! of it is written. Two tenants, both backends.

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

fn bundle(name: &str) -> WorkspaceImport {
    let now = Utc::now();
    let ws = WorkspaceId(uuid::Uuid::new_v4());
    let author = member(ws, "author");
    let ch = ChannelId(uuid::Uuid::new_v4());
    let t = thread(ch, Some(author.id));
    let msg = MessageId(uuid::Uuid::new_v4());
    WorkspaceImport {
        workspace: Workspace {
            id: ws,
            name: name.into(),
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        },
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
        channel_members: vec![ChannelMember {
            channel_id: ch,
            member_id: author.id,
            role: ChannelMemberRole::Member,
            created_at: now,
        }],
        messages: vec![Message {
            id: msg,
            thread_id: t.id,
            author_id: author.id,
            body: "hi".into(),
            metadata: serde_json::json!({}),
            content: None,
            posted_at: now,
            edited_at: None,
            tombstoned_at: None,
        }],
        message_edits: vec![MessageEdit {
            id: 0,
            message_id: msg,
            editor_id: author.id,
            body_before: "h".into(),
            body_after: "hi".into(),
            edited_at: now,
        }],
        pins: vec![Pin {
            thread_id: t.id,
            message_id: msg,
            member_id: author.id,
            created_at: now,
        }],
        references: vec![Reference {
            id: uuid::Uuid::new_v4(),
            src_kind: RefSide::Thread,
            src_id: t.id.0,
            dst_kind: RefSide::Message,
            dst_id: msg.0,
            relation: "about".into(),
            created_at: now,
        }],
        threads: vec![t],
        members: vec![author],
    }
}

type Tamper = fn(&mut WorkspaceImport, &WorkspaceImport);

/// One bundle edit per reference kind, each pointing tenant B's bundle at
/// tenant A's rows.
fn tampers() -> Vec<(&'static str, Tamper)> {
    vec![
        ("member workspace", |b, a| {
            b.members[0].workspace_id = a.workspace.id
        }),
        ("channel workspace", |b, a| {
            b.channels[0].workspace_id = a.workspace.id
        }),
        ("channel membership channel", |b, a| {
            b.channel_members[0].channel_id = a.channels[0].id
        }),
        ("channel membership member", |b, a| {
            b.channel_members[0].member_id = a.members[0].id
        }),
        ("thread channel", |b, a| {
            b.threads[0].channel_id = a.channels[0].id
        }),
        ("thread parent", |b, a| {
            b.threads[0].parent_thread_id = Some(a.threads[0].id)
        }),
        ("thread holder", |b, a| {
            b.threads[0].assignee_id = Some(a.members[0].id)
        }),
        ("thread owner", |b, a| {
            b.threads[0].owner_id = Some(a.members[0].id)
        }),
        ("thread creator", |b, a| {
            b.threads[0].created_by = Some(a.members[0].id)
        }),
        ("message thread", |b, a| {
            b.messages[0].thread_id = a.threads[0].id
        }),
        ("message author", |b, a| {
            b.messages[0].author_id = a.members[0].id
        }),
        ("edit message", |b, a| {
            b.message_edits[0].message_id = a.messages[0].id
        }),
        ("edit editor", |b, a| {
            b.message_edits[0].editor_id = a.members[0].id
        }),
        ("pin thread", |b, a| b.pins[0].thread_id = a.threads[0].id),
        ("pin message", |b, a| {
            b.pins[0].message_id = a.messages[0].id
        }),
        ("pin member", |b, a| b.pins[0].member_id = a.members[0].id),
        ("reference source", |b, a| {
            b.references[0].src_id = a.threads[0].id.0
        }),
        ("reference target", |b, a| {
            b.references[0].dst_id = a.messages[0].id.0
        }),
    ]
}

async fn suite(store: &dyn Store) {
    let a = bundle("a");
    store
        .import_workspace(&a, WorkspaceOrigin::Operator)
        .await
        .unwrap();
    let clean = bundle("clean");
    store
        .import_workspace(&clean, WorkspaceOrigin::Operator)
        .await
        .unwrap();

    for (kind, tamper) in tampers() {
        let mut b = bundle(kind);
        tamper(&mut b, &a);
        let err = store
            .import_workspace(&b, WorkspaceOrigin::Operator)
            .await
            .unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidInput(_)),
            "{kind}: expected a validation error, got {err:?}"
        );
        assert!(
            store.get_workspace(b.workspace.id).await.is_err(),
            "{kind}: the refused bundle's workspace was written"
        );
        assert!(
            store
                .list_messages(b.threads[0].id, 10)
                .await
                .unwrap()
                .is_empty(),
            "{kind}: the refused bundle's messages were written"
        );
    }

    // Tenant A is untouched by every refused bundle.
    let msgs = store.list_messages(a.threads[0].id, 10).await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].author_id, a.members[0].id);
    assert_eq!(
        store.list_members(a.workspace.id).await.unwrap().len(),
        1,
        "no member was written into tenant A"
    );
    assert_eq!(store.list_channels(a.workspace.id).await.unwrap().len(), 1);
}

/// A bundle that names nothing outside itself still fails when a reference
/// points at nothing at all, rather than at another tenant.
async fn dangling(store: &dyn Store) {
    let mut b = bundle("dangling");
    b.messages[0].author_id = MemberId(uuid::Uuid::new_v4());
    let err = store
        .import_workspace(&b, WorkspaceOrigin::Operator)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidInput(_)), "{err:?}");
    assert!(store.get_workspace(b.workspace.id).await.is_err());
}

fn restore_event(ws: WorkspaceId) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(ws),
        actor_id: None,
        action: "workspace.import".into(),
        target_kind: Some("workspace".into()),
        target_id: Some(ws.0),
        metadata: serde_json::Value::Null,
    }
}

/// A forced restore erases the workspace it names before it writes, in one
/// transaction. A bundle refused after that erase, whether by the reference
/// check or by an id already held in another tenant, leaves both tenants as
/// they were.
async fn forced(store: &dyn Store) {
    let a = bundle("forced-a");
    store
        .import_workspace(&a, WorkspaceOrigin::Operator)
        .await
        .unwrap();
    let b = bundle("forced-b");
    store
        .import_workspace(&b, WorkspaceOrigin::Operator)
        .await
        .unwrap();

    let mut reaching = b.clone();
    reaching.messages[0].author_id = a.members[0].id;
    let err = store
        .import_workspace_audited(
            &reaching,
            true,
            WorkspaceOrigin::Operator,
            restore_event(b.workspace.id),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidInput(_)), "{err:?}");

    let mut colliding = b.clone();
    let mut stolen = member(b.workspace.id, "stolen");
    stolen.id = a.members[0].id;
    colliding.members.push(stolen);
    assert!(
        store
            .import_workspace_audited(
                &colliding,
                true,
                WorkspaceOrigin::Operator,
                restore_event(b.workspace.id),
            )
            .await
            .is_err(),
        "a restore reusing another tenant's member id was accepted"
    );

    for t in [&a, &b] {
        assert!(store.get_workspace(t.workspace.id).await.is_ok());
        let members = store.list_members(t.workspace.id).await.unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].id, t.members[0].id);
        let msgs = store.list_messages(t.threads[0].id, 10).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].author_id, t.members[0].id);
    }
}

#[tokio::test]
async fn a_restore_reaching_into_another_tenant_is_refused_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    suite(&store).await;
    dangling(&store).await;
    forced(&store).await;
}

#[tokio::test]
async fn a_restore_reaching_into_another_tenant_is_refused_on_postgres() {
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
    let store = PostgresStore::for_tests(pool);
    suite(&store).await;
    dangling(&store).await;
    forced(&store).await;
}
