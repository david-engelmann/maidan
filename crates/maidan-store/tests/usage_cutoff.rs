//! The hard allowance cutoff's store side, on both backends: a workspace's
//! origin is recorded by the sign-up transaction, and only an open sign-up
//! workspace's crossings are listed as cut off — never an invited or
//! operator-created one's, and never another month's.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn signed_up(store: &dyn Store, who: &str, open: bool) -> WorkspaceId {
    store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: Utc::now() + Duration::from_secs(3600),
                open,
            },
            Box::new(|core| NewAuditEvent {
                scope: AuditScope::Workspace(core.workspace.id),
                actor_id: Some(core.member.id),
                action: "workspace.signup".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(core.workspace.id.0),
                metadata: serde_json::json!({}),
            }),
            Box::new(|session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: serde_json::json!({}),
            }),
        )
        .await
        .unwrap()
        .workspace
        .id
}

async fn over(store: &dyn Store, ws: WorkspaceId, meter: UsageMeter, month: &str) {
    let mark = OverAllowance {
        workspace_id: ws,
        meter,
        month: month.into(),
        quantity: 11,
        allowance: 10,
        marked_at: Utc::now(),
    };
    store.mark_over_allowance(&mark, &[]).await.unwrap();
}

/// A one-thread workspace, imported with `origin`: (workspace, member, thread).
async fn imported(
    store: &dyn Store,
    origin: WorkspaceOrigin,
) -> (WorkspaceImport, MemberId, ThreadId) {
    let bundle = bundle(WorkspaceId(uuid::Uuid::new_v4()));
    store.import_workspace(&bundle, origin).await.unwrap();
    let member = bundle.members[0].id;
    let thread = bundle.threads[0].id;
    (bundle, member, thread)
}

fn bundle(ws: WorkspaceId) -> WorkspaceImport {
    let now = Utc::now();
    let member = MemberId(uuid::Uuid::new_v4());
    let channel = ChannelId(uuid::Uuid::new_v4());
    WorkspaceImport {
        workspace: Workspace {
            id: ws,
            name: "imported".into(),
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        },
        members: vec![Member {
            id: member,
            workspace_id: ws,
            handle: "ada".into(),
            display_name: None,
            kind: MemberKind::Human,
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        }],
        channels: vec![Channel {
            id: channel,
            workspace_id: ws,
            name: "general".into(),
            topic: None,
            private: false,
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        }],
        channel_members: vec![],
        threads: vec![Thread {
            id: ThreadId(uuid::Uuid::new_v4()),
            channel_id: channel,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            state: ThreadState::Open,
            assignee_id: None,
            assignment_expires_at: None,
            claim_lease_id: None,
            work_started_at: None,
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
        }],
        messages: vec![],
        message_edits: vec![],
        pins: vec![],
        references: vec![],
    }
}

fn post(thread: ThreadId, author: MemberId, body: &str) -> NewMessage {
    NewMessage {
        thread_id: thread,
        author_id: author,
        body: body.into(),
        metadata: serde_json::json!({}),
        content: None,
    }
}

fn edit(body: &str) -> EditMessage {
    EditMessage {
        body: body.into(),
        metadata: serde_json::json!({}),
        content: None,
    }
}

/// The stored-bytes cutoff is held at the store's writes, so every writer
/// (route, tool, seed, A2A send, inbound) is refused the same way; and an
/// import carries an origin, so it cannot turn an open workspace into an
/// operator one.
async fn run_write_suite(store: &dyn Store) {
    let (cut, cut_member, cut_thread) = imported(store, WorkspaceOrigin::Open).await;
    let (_, other_member, other_thread) = imported(store, WorkspaceOrigin::Open).await;
    let cut = cut.workspace.id;
    assert_eq!(
        store.workspace_origin(cut).await.unwrap(),
        WorkspaceOrigin::Open
    );
    let before = store
        .post_message(post(cut_thread, cut_member, "short"))
        .await
        .unwrap();

    store
        .usage_meters()
        .set_cut_off([(cut, UsageMeter::StoredBytes)]);
    let refused = store
        .post_message(post(cut_thread, cut_member, "more"))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, StoreError::OverAllowance(UsageMeter::StoredBytes)),
        "{refused:?}"
    );
    let refused = store
        .post_message_with_event(post(cut_thread, cut_member, "more"), None)
        .await
        .unwrap_err();
    assert!(
        matches!(refused, StoreError::OverAllowance(_)),
        "{refused:?}"
    );
    store
        .post_message(post(other_thread, other_member, "fine"))
        .await
        .expect("another workspace still posts");

    let grown = store
        .edit_message(before.id, cut_member, edit("much longer than before"))
        .await
        .unwrap_err();
    assert!(matches!(grown, StoreError::OverAllowance(_)), "{grown:?}");
    store
        .edit_message(before.id, cut_member, edit("tiny"))
        .await
        .expect("an edit that shrinks still runs");

    let upload = NewArtifact {
        sha256: "b".repeat(64),
        size_bytes: 3,
        mime_type: Some("text/plain".into()),
        filename: None,
        kind: ArtifactKind::Transcript,
        uploaded_by: Some(cut_member),
    };
    let refused = store
        .upsert_artifact_with_event(upload, Some(cut))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, StoreError::OverAllowance(_)),
        "{refused:?}"
    );

    // Import while cut off is refused, for the caller and for a restore
    // target; an uncut open caller gets an open workspace.
    let refused = maidan_store::usage_meters::import_origin(store, false, cut, None)
        .await
        .unwrap_err();
    assert!(
        matches!(refused, StoreError::OverAllowance(_)),
        "{refused:?}"
    );
    let other_ws = store.get_member(other_member).await.unwrap().workspace_id;
    let refused = maidan_store::usage_meters::import_origin(store, true, other_ws, Some(cut))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, StoreError::OverAllowance(_)),
        "{refused:?}"
    );
    assert_eq!(
        maidan_store::usage_meters::import_origin(store, false, other_ws, None)
            .await
            .unwrap(),
        WorkspaceOrigin::Open,
        "mode=new from an open workspace makes an open workspace"
    );
    assert_eq!(
        maidan_store::usage_meters::import_origin(store, true, other_ws, None)
            .await
            .unwrap(),
        WorkspaceOrigin::Operator
    );

    // A forced restore over an open workspace keeps it open, and keeps its
    // allowance crossing, even when the caller asked for operator.
    over(store, cut, UsageMeter::StoredBytes, "2026-07").await;
    let restore = bundle(cut);
    store
        .import_workspace_audited(
            &restore,
            true,
            WorkspaceOrigin::Operator,
            NewAuditEvent {
                scope: AuditScope::Workspace(cut),
                actor_id: None,
                action: "workspace.import".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(cut.0),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store.workspace_origin(cut).await.unwrap(),
        WorkspaceOrigin::Open
    );
    assert!(
        store
            .list_usage_cut_off("2026-07")
            .await
            .unwrap()
            .contains(&(cut, UsageMeter::StoredBytes)),
        "a restore does not clear the cutoff"
    );
    store.usage_meters().set_cut_off([]);
}

async fn run_suite(store: &dyn Store) {
    run_write_suite(store).await;
    let open_a = signed_up(store, "ada", true).await;
    let open_b = signed_up(store, "bea", true).await;
    let invited = signed_up(store, "cy", false).await;
    let operator = store
        .create_workspace(NewWorkspace { name: "ops".into() })
        .await
        .unwrap()
        .id;

    assert!(
        store
            .list_usage_cut_off("2026-10")
            .await
            .unwrap()
            .is_empty(),
        "nothing is cut off before a crossing"
    );
    for ws in [open_a, invited, operator] {
        over(store, ws, UsageMeter::Requests, "2026-10").await;
    }
    over(store, open_b, UsageMeter::McpCalls, "2026-09").await;

    assert_eq!(
        store.list_usage_cut_off("2026-10").await.unwrap(),
        vec![(open_a, UsageMeter::Requests)],
        "only the open workspace past it, on its own meter; B's usage is B's"
    );
    assert_eq!(
        store.list_usage_cut_off("2026-09").await.unwrap(),
        vec![(open_b, UsageMeter::McpCalls)],
        "a month's crossings stay in that month"
    );
    assert!(store
        .list_usage_cut_off("2026-11")
        .await
        .unwrap()
        .is_empty());

    // Every meter, not just requests: past each one, the open workspace is
    // listed on each, and the invited and operator ones on none.
    for ws in [open_a, invited, operator] {
        for meter in UsageMeter::ALL {
            over(store, ws, meter, "2026-08").await;
        }
    }
    let mut listed = store.list_usage_cut_off("2026-08").await.unwrap();
    listed.sort_by_key(|(_, meter)| meter.as_str());
    let mut expected: Vec<_> = UsageMeter::ALL.into_iter().map(|m| (open_a, m)).collect();
    expected.sort_by_key(|(_, meter)| meter.as_str());
    assert_eq!(
        listed, expected,
        "each meter cuts off the open workspace only"
    );

    // A new open workspace signs up while another is cut off, and starts
    // with nothing cut off.
    let fresh = signed_up(store, "dee", true).await;
    assert!(store
        .list_usage_cut_off("2026-08")
        .await
        .unwrap()
        .iter()
        .all(|(ws, _)| *ws != fresh));
}

#[tokio::test]
async fn only_open_signup_workspaces_past_their_allowance_are_cut_off_sqlite() {
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
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn only_open_signup_workspaces_past_their_allowance_are_cut_off_postgres() {
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
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::for_tests(pool)).await;
}
