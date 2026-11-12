//! SEC-018 and SEC-012: what a member left running ends with its authority,
//! on both backends.
//!
//! A task schedule fires as its creator, a share ticket shows a channel on its
//! owner's and creator's authority, and an invite admits someone on its
//! inviter's word. Deactivating or deprovisioning the member used to leave all
//! three live: the schedule kept opening threads as a member who could no
//! longer sign in, the ticket kept the channel open to whoever held the link,
//! and the invite still let its recipient in. Now ending the member's
//! authority pauses its schedules and revokes its tickets and open invites in
//! the same transaction, each with an audit row naming the reason. Another
//! member's, and another workspace's, are untouched.

use chrono::{Duration, Utc};
use maidan_auth::hash_secret;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewAuditEvent, NewChannel, NewMember, NewShareTicket,
    NewTaskSchedule, NewWorkspace, NewWorkspaceInvite, ShareTicketId, TaskScheduleId, WorkspaceId,
    WorkspaceInviteId, WorkspaceInviteRole,
};

fn event(ws: WorkspaceId, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: maidan_types::AuditScope::Workspace(ws),
        actor_id: None,
        action: action.into(),
        target_kind: None,
        target_id: None,
        metadata: serde_json::json!({}),
    }
}

/// What one member left running: a schedule, a ticket it owns and made, and
/// an open invite.
struct Left {
    schedule: TaskScheduleId,
    ticket: ShareTicketId,
    invite: WorkspaceInviteId,
}

struct Tenant {
    ws: WorkspaceId,
    channel: ChannelId,
}

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "incident".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap()
        .id;
    Tenant { ws, channel }
}

async fn provision(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    let (member, _) = store
        .scim_provision_audited(
            NewMember {
                workspace_id: ws,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Human,
            },
            Some(handle),
            true,
            Box::new(move |_| event(ws, "scim.user.create")),
        )
        .await
        .unwrap();
    member.id
}

async fn ticket(
    store: &dyn Store,
    t: &Tenant,
    owner: MemberId,
    created_by: MemberId,
    secret: &str,
) -> ShareTicketId {
    store
        .create_share_ticket(NewShareTicket {
            workspace_id: t.ws,
            channel_id: t.channel,
            owner_id: owner,
            created_by,
            token_hash: hash_secret(secret),
            expires_at: Utc::now() + Duration::hours(1),
            artifact_shas: Vec::new(),
        })
        .await
        .unwrap()
        .id
}

async fn leave_running(store: &dyn Store, t: &Tenant, member: MemberId, tag: &str) -> Left {
    let schedule = store
        .create_task_schedule(NewTaskSchedule {
            workspace_id: t.ws,
            channel_id: t.channel,
            title: format!("{tag} nightly"),
            interval_secs: Some(3600),
            next_run_at: Utc::now() + Duration::hours(1),
            created_by: member,
            recipe_id: None,
        })
        .await
        .unwrap()
        .id;
    let ticket = ticket(store, t, member, member, &format!("{tag}-ticket")).await;
    let ws = t.ws;
    let invite = store
        .create_workspace_invite_audited(
            NewWorkspaceInvite {
                workspace_id: t.ws,
                email: format!("{tag}@example.test"),
                role: WorkspaceInviteRole::Member,
                created_by: member,
                token_hash: hash_secret(&format!("{tag}-invite")),
                expires_at: Utc::now() + Duration::hours(1),
            },
            Box::new(move |_| event(ws, "workspace_invite.create")),
        )
        .await
        .unwrap()
        .id;
    Left {
        schedule,
        ticket,
        invite,
    }
}

/// `(schedule active, ticket live, invite live)`.
async fn state_of(store: &dyn Store, left: &Left) -> (bool, bool, bool) {
    (
        store.get_task_schedule(left.schedule).await.unwrap().active,
        store
            .get_share_ticket(left.ticket)
            .await
            .unwrap()
            .revoked_at
            .is_none(),
        store
            .get_workspace_invite(left.invite)
            .await
            .unwrap()
            .revoked_at
            .is_none(),
    )
}

async fn run_suite(store: &dyn Store) {
    let a = tenant(store, "a").await;
    let b = tenant(store, "b").await;
    let deactivated = provision(store, a.ws, "deactivated").await;
    let deprovisioned = provision(store, a.ws, "deprovisioned").await;
    let stays = provision(store, a.ws, "stays").await;
    let elsewhere = provision(store, b.ws, "elsewhere").await;

    let by_deactivated = leave_running(store, &a, deactivated, "deactivated").await;
    let by_deprovisioned = leave_running(store, &a, deprovisioned, "deprovisioned").await;
    let by_stays = leave_running(store, &a, stays, "stays").await;
    let by_elsewhere = leave_running(store, &b, elsewhere, "elsewhere").await;
    // Owned by a member who stays, made by one who goes: the maker's
    // authority is in it too.
    let made_for_stays = ticket(store, &a, stays, deactivated, "made-for-stays").await;

    store
        .scim_update_user_audited(
            a.ws,
            deactivated,
            None,
            Some("deactivated"),
            false,
            event(a.ws, "deactivate"),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(store
        .scim_deprovision_audited(a.ws, deprovisioned, event(a.ws, "delete"))
        .await
        .unwrap());

    for (left, who) in [
        (&by_deactivated, "deactivated"),
        (&by_deprovisioned, "deprovisioned"),
    ] {
        assert_eq!(
            state_of(store, left).await,
            (false, false, false),
            "the {who} member's schedule, ticket and invite outlived it"
        );
    }
    assert!(
        store
            .get_share_ticket(made_for_stays)
            .await
            .unwrap()
            .revoked_at
            .is_some(),
        "a ticket its maker can no longer stand behind stays live"
    );
    assert_eq!(
        state_of(store, &by_stays).await,
        (true, true, true),
        "another member's were ended"
    );
    assert_eq!(
        state_of(store, &by_elsewhere).await,
        (true, true, true),
        "another workspace's were ended"
    );

    let audit = store.list_audit(500).await.unwrap();
    let recorded = |action: &str, target: uuid::Uuid, reason: &str| {
        audit.iter().any(|row| {
            row.action == action
                && row.target_id == Some(target)
                && row.metadata["reason"] == reason
        })
    };
    for (left, reason) in [
        (&by_deactivated, "scim_deactivate"),
        (&by_deprovisioned, "scim_deprovision"),
    ] {
        assert!(recorded("task_schedule.pause", left.schedule.0, reason));
        assert!(recorded("share_ticket.revoke", left.ticket.0, reason));
        assert!(recorded("workspace_invite.revoke", left.invite.0, reason));
    }
}

#[tokio::test]
async fn ending_a_member_ends_its_schedules_tickets_and_invites_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
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
async fn ending_a_member_ends_its_schedules_tickets_and_invites_postgres() {
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
