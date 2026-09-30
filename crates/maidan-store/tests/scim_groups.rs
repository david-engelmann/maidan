//! SCIM groups and user renames, on both backends, with two workspaces.
//!
//! A group, its members and a rename stay inside one workspace: a member of
//! another workspace, or a member the identity provider did not provision, is
//! refused by the store and again by the membership row's foreign keys. A
//! rename keeps the member's id and refuses a handle already taken.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, Member, MemberId, MemberKind, NewAuditEvent, NewMember, NewScimGroup, NewWorkspace,
    ScimGroupChange, ScimGroupId, ScimMembersOp, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

// These tests are about where a group and its members may live, not about
// which workspace an audit row names (`audit_workspace` covers that).
fn event(action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Instance,
        actor_id: None,
        action: action.into(),
        target_kind: None,
        target_id: None,
        metadata: serde_json::json!({}),
    }
}

async fn provision(store: &dyn Store, ws: WorkspaceId, handle: &str) -> Member {
    store
        .scim_provision_audited(
            NewMember {
                workspace_id: ws,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Human,
            },
            None,
            true,
            Box::new(|_| event("scim.user.create")),
        )
        .await
        .expect("provision")
        .0
}

fn change(members: Vec<ScimMembersOp>) -> ScimGroupChange {
    ScimGroupChange {
        members,
        ..ScimGroupChange::default()
    }
}

async fn member_ids(store: &dyn Store, ws: WorkspaceId, id: ScimGroupId) -> Vec<MemberId> {
    store
        .get_scim_group(ws, id)
        .await
        .expect("get")
        .expect("group")
        .members
        .into_iter()
        .map(|m| m.member_id)
        .collect()
}

/// A raw membership insert, bypassing the store's check: what the foreign keys
/// alone decide.
type RawInsert<'a> = &'a (dyn Fn(Uuid, Uuid, Uuid) -> Pending<'a> + Sync);
type Pending<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>>;

async fn run_suite(store: &dyn Store, raw_insert: RawInsert<'_>) {
    let ws_a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap()
        .id;
    let ws_b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let alice = provision(store, ws_a, "alice").await;
    let amir = provision(store, ws_a, "amir").await;
    let bea = provision(store, ws_b, "bea").await;
    let plain = store
        .create_member(NewMember {
            workspace_id: ws_a,
            handle: "plain".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();

    // Create, with a member.
    let eng = store
        .scim_create_group_audited(
            NewScimGroup {
                workspace_id: ws_a,
                display_name: "Engineering".into(),
                external_id: Some("okta-g1".into()),
                members: vec![alice.id, alice.id],
            },
            Box::new(|_| event("scim.group.create")),
        )
        .await
        .unwrap();
    assert_eq!(eng.display_name, "Engineering");
    assert_eq!(eng.external_id.as_deref(), Some("okta-g1"));
    assert_eq!(eng.members.len(), 1);
    assert_eq!(eng.members[0].member_id, alice.id);
    assert_eq!(eng.members[0].handle, "alice");

    // Another workspace's user, and a member the IdP did not provision, are
    // refused, and the refused create leaves no group behind.
    for outsider in [bea.id, plain.id] {
        let refused = store
            .scim_create_group_audited(
                NewScimGroup {
                    workspace_id: ws_a,
                    display_name: "Leaky".into(),
                    external_id: None,
                    members: vec![amir.id, outsider],
                },
                Box::new(|_| event("scim.group.create")),
            )
            .await;
        assert!(
            matches!(refused, Err(StoreError::InvalidInput(_))),
            "{refused:?}"
        );
    }
    let groups = store.list_scim_groups(ws_a).await.unwrap();
    assert_eq!(groups.len(), 1, "a refused create writes nothing");

    // Add is idempotent and reports the net change.
    let write = store
        .scim_update_group_audited(
            ws_a,
            eng.id,
            change(vec![ScimMembersOp::Add(vec![amir.id, alice.id])]),
            Box::new(|_| event("scim.group.update")),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(write.added, vec![amir.id]);
    assert!(write.removed.is_empty());
    assert_eq!(write.group.members.len(), 2);

    // Adding another workspace's user fails the whole change: the rename in
    // the same change and the removal before it do not happen either.
    let refused = store
        .scim_update_group_audited(
            ws_a,
            eng.id,
            ScimGroupChange {
                display_name: Some("Renamed".into()),
                external_id: None,
                members: vec![
                    ScimMembersOp::Remove(vec![alice.id]),
                    ScimMembersOp::Add(vec![bea.id]),
                ],
            },
            Box::new(|_| event("scim.group.update")),
        )
        .await;
    assert!(matches!(refused, Err(StoreError::InvalidInput(_))));
    let unchanged = store.get_scim_group(ws_a, eng.id).await.unwrap().unwrap();
    assert_eq!(unchanged.display_name, "Engineering");
    assert_eq!(unchanged.members.len(), 2);

    // Remove, then replace.
    let write = store
        .scim_update_group_audited(
            ws_a,
            eng.id,
            change(vec![ScimMembersOp::Remove(vec![alice.id, bea.id])]),
            Box::new(|_| event("scim.group.update")),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(write.removed, vec![alice.id]);
    assert_eq!(member_ids(store, ws_a, eng.id).await, vec![amir.id]);
    let write = store
        .scim_update_group_audited(
            ws_a,
            eng.id,
            change(vec![ScimMembersOp::Replace(vec![alice.id])]),
            Box::new(|_| event("scim.group.update")),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(write.added, vec![alice.id]);
    assert_eq!(write.removed, vec![amir.id]);
    // An add then a remove of the same member nets to nothing.
    let write = store
        .scim_update_group_audited(
            ws_a,
            eng.id,
            change(vec![
                ScimMembersOp::Add(vec![amir.id]),
                ScimMembersOp::Remove(vec![amir.id]),
            ]),
            Box::new(|_| event("scim.group.update")),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(write.added.is_empty() && write.removed.is_empty());

    // Attributes: rename the group, clear its external id.
    let write = store
        .scim_update_group_audited(
            ws_a,
            eng.id,
            ScimGroupChange {
                display_name: Some("Eng".into()),
                external_id: Some(None),
                members: vec![],
            },
            Box::new(|_| event("scim.group.update")),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(write.group.display_name, "Eng");
    assert!(write.group.external_id.is_none());
    assert_eq!(write.group.members.len(), 1, "no member op, no change");

    // Workspace B cannot see, change or delete workspace A's group.
    assert!(store.get_scim_group(ws_b, eng.id).await.unwrap().is_none());
    assert!(store.list_scim_groups(ws_b).await.unwrap().is_empty());
    assert!(store
        .scim_update_group_audited(
            ws_b,
            eng.id,
            change(vec![ScimMembersOp::Replace(vec![])]),
            Box::new(|_| event("scim.group.update")),
        )
        .await
        .unwrap()
        .is_none());
    assert!(!store
        .scim_delete_group_audited(ws_b, eng.id, event("scim.group.delete"))
        .await
        .unwrap());
    assert_eq!(member_ids(store, ws_a, eng.id).await, vec![alice.id]);

    // The foreign keys refuse a cross-workspace membership on their own: the
    // member's workspace must be the group's.
    assert!(
        !raw_insert(eng.id.0, bea.id.0, ws_a.0).await,
        "a member of workspace B under workspace A"
    );
    assert!(
        !raw_insert(eng.id.0, bea.id.0, ws_b.0).await,
        "workspace A's group under workspace B"
    );
    assert!(
        !raw_insert(eng.id.0, plain.id.0, ws_a.0).await,
        "a member the IdP did not provision"
    );
    assert!(raw_insert(eng.id.0, amir.id.0, ws_a.0).await);

    // A rename keeps the id; the group shows the new handle at once.
    let renamed = store
        .scim_update_user_audited(
            ws_a,
            alice.id,
            Some("alice.new"),
            None,
            true,
            event("scim.user.update"),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.member_id, alice.id);
    let member = store.get_member(alice.id).await.unwrap();
    assert_eq!(member.handle, "alice.new");
    assert_eq!(
        store
            .get_member_by_handle(ws_a, "alice.new")
            .await
            .unwrap()
            .id,
        alice.id
    );
    assert!(store.get_member_by_handle(ws_a, "alice").await.is_err());
    let eng_now = store.get_scim_group(ws_a, eng.id).await.unwrap().unwrap();
    assert!(eng_now
        .members
        .iter()
        .any(|m| m.member_id == alice.id && m.handle == "alice.new"));

    // A handle another member holds is a conflict, and nothing changes.
    let taken = store
        .scim_update_user_audited(
            ws_a,
            alice.id,
            Some("amir"),
            Some("changed"),
            true,
            event("scim.user.update"),
        )
        .await;
    assert!(matches!(taken, Err(StoreError::Conflict(_))), "{taken:?}");
    assert_eq!(
        store.get_member(alice.id).await.unwrap().handle,
        "alice.new"
    );
    assert!(store
        .get_scim_user(alice.id)
        .await
        .unwrap()
        .unwrap()
        .external_id
        .is_none());
    // The same handle in another workspace is not taken.
    store
        .scim_update_user_audited(ws_b, bea.id, Some("amir"), None, true, event("u"))
        .await
        .unwrap()
        .unwrap();

    // Workspace B cannot rename workspace A's user.
    assert!(store
        .scim_update_user_audited(ws_b, alice.id, Some("stolen"), None, false, event("u"))
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store.get_member(alice.id).await.unwrap().handle,
        "alice.new"
    );
    assert!(store.get_scim_user(alice.id).await.unwrap().unwrap().active);

    // Deprovisioning a user takes it out of every group.
    store
        .scim_deprovision_audited(ws_a, amir.id, event("scim.user.delete"))
        .await
        .unwrap();
    assert_eq!(member_ids(store, ws_a, eng.id).await, vec![alice.id]);

    // Delete.
    assert!(store
        .scim_delete_group_audited(ws_a, eng.id, event("scim.group.delete"))
        .await
        .unwrap());
    assert!(store.get_scim_group(ws_a, eng.id).await.unwrap().is_none());
    assert!(!store
        .scim_delete_group_audited(ws_a, eng.id, event("scim.group.delete"))
        .await
        .unwrap());

    let actions: Vec<String> = store
        .list_audit(200)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.action)
        .collect();
    assert_eq!(
        actions.iter().filter(|a| *a == "scim.group.create").count(),
        1,
        "refused creates record nothing"
    );
    assert_eq!(
        actions.iter().filter(|a| *a == "scim.group.update").count(),
        5
    );
    assert_eq!(
        actions.iter().filter(|a| *a == "scim.group.delete").count(),
        1
    );
}

#[tokio::test]
async fn scim_groups_and_renames_stay_in_their_workspace_sqlite() {
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
    let store = SqliteStore::for_tests(pool.clone());
    let raw = |group: Uuid, member: Uuid, ws: Uuid| -> Pending<'_> {
        let pool = pool.clone();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO maidan_scim_group_members (group_id, member_id, workspace_id, added_at)
                 VALUES (?, ?, ?, '2026-01-01T00:00:00Z') ON CONFLICT DO NOTHING",
            )
            .bind(group)
            .bind(member)
            .bind(ws)
            .execute(&pool)
            .await
            .is_ok()
        })
    };
    run_suite(&store, &raw).await;
}

#[tokio::test]
async fn scim_groups_and_renames_stay_in_their_workspace_postgres() {
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
    let store = PostgresStore::for_tests(pool.clone());
    let raw = |group: Uuid, member: Uuid, ws: Uuid| -> Pending<'_> {
        let pool = pool.clone();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO maidan_scim_group_members (group_id, member_id, workspace_id)
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(group)
            .bind(member)
            .bind(ws)
            .execute(&pool)
            .await
            .is_ok()
        })
    };
    run_suite(&store, &raw).await;
}
