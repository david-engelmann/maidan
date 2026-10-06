//! Every audit row belongs to a workspace, on both backends.
//!
//! Before, a row carried none: the workspace audit view guessed from the
//! actor's membership, so rows with no actor (an app-token mint, a SCIM
//! deprovision's token revokes, a result-delivery attempt) were in no
//! workspace's view, and a legal hold anywhere froze audit pruning for every
//! workspace, because pruning could not tell whose a row was.

use chrono::{Duration, Utc};
use maidan_auth::hash_secret;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::{
    AuditScope, Member, MemberKind, NewApiToken, NewAuditEvent, NewChannel, NewMember, NewThread,
    NewWorkspace, WorkspaceId,
};
use sqlx::{PgPool, SqlitePool};

enum Db {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl Db {
    fn store(&self) -> Box<dyn Store> {
        match self {
            Db::Sqlite(pool) => Box::new(SqliteStore::for_tests(pool.clone())),
            Db::Postgres(pool) => Box::new(PostgresStore::for_tests(pool.clone())),
        }
    }

    async fn backdate_audit(&self, days: i64) {
        let at = Utc::now() - Duration::days(days);
        match self {
            Db::Sqlite(pool) => {
                sqlx::query("UPDATE maidan_audit SET occurred_at = ?")
                    .bind(at)
                    .execute(pool)
                    .await
                    .expect("backdate");
            }
            Db::Postgres(pool) => {
                sqlx::query("UPDATE maidan_audit SET occurred_at = $1")
                    .bind(at)
                    .execute(pool)
                    .await
                    .expect("backdate");
            }
        }
    }

    /// Put the schema back to before 0123 and apply it again, so rows written
    /// without a workspace go through the real backfill.
    async fn reapply_audit_workspace_migration(&self) {
        const UNDO: &str = "DROP INDEX idx_audit_workspace;
             ALTER TABLE maidan_audit DROP COLUMN workspace_id;
             DELETE FROM maidan_migrations WHERE version = 123;";
        match self {
            Db::Sqlite(pool) => {
                sqlx::raw_sql(UNDO).execute(pool).await.expect("undo 0123");
                run_sqlite_migrations(pool).await.expect("reapply");
            }
            Db::Postgres(pool) => {
                sqlx::raw_sql(UNDO).execute(pool).await.expect("undo 0123");
                run_postgres_migrations(pool).await.expect("reapply");
            }
        }
    }
}

async fn tenant(store: &dyn Store, name: &str) -> Member {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace");
    store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-admin"),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member")
}

/// A row as an actor-less writer records it: outside any request, so nothing
/// but the stamp says whose it is.
fn actorless(scope: AuditScope, action: &str, target_kind: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope,
        actor_id: None,
        action: action.into(),
        target_kind: Some(target_kind.into()),
        target_id: Some(uuid::Uuid::now_v7()),
        metadata: serde_json::json!({}),
    }
}

async fn ids_in_view(store: &dyn Store, ws: WorkspaceId) -> Vec<i64> {
    let mut ids: Vec<i64> = store
        .list_audit_for_workspace(ws, 500)
        .await
        .expect("view")
        .iter()
        .map(|row| row.id)
        .collect();
    ids.sort_unstable();
    ids
}

/// Rows the tenant's own writers leave with no actor: an app-token mint, a SCIM
/// deprovision (whose token revokes the store writes itself), a result-delivery
/// attempt.
async fn actorless_rows(store: &dyn Store, admin: &Member) -> Vec<i64> {
    let ws = admin.workspace_id;
    let mut ids = Vec::new();
    for (action, kind) in [
        ("app_token.mint", "api_token"),
        ("result_delivery.attempt", "result_delivery"),
    ] {
        let row = store
            .append_audit(actorless(AuditScope::Workspace(ws), action, kind))
            .await
            .expect("append");
        assert_eq!(row.workspace_id, Some(ws), "the stamp is read back");
        ids.push(row.id);
    }

    let (user, _) = store
        .scim_provision_audited(
            NewMember {
                workspace_id: ws,
                handle: format!("{}-scim", admin.handle),
                display_name: None,
                kind: MemberKind::Human,
            },
            Some("ext"),
            true,
            Box::new(move |(member, _)| NewAuditEvent {
                scope: AuditScope::Workspace(member.workspace_id),
                actor_id: None,
                action: "scim.user.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(member.id.0),
                metadata: serde_json::json!({}),
            }),
        )
        .await
        .expect("provision");
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: user.id,
            app_installation_id: None,
            token_hash: hash_secret(&format!("{}-secret", admin.handle)),
            label: None,
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .expect("token");
    store
        .scim_deprovision_audited(
            ws,
            user.id,
            actorless(AuditScope::Workspace(ws), "scim.user.delete", "member"),
        )
        .await
        .expect("deprovision");

    let all = store.list_audit(500).await.expect("all");
    let revoke = all
        .iter()
        .find(|row| {
            row.action == "token.revoke" && row.metadata["workspace_id"] == ws.0.to_string()
        })
        .expect("the deprovision revoked the member's token");
    assert_eq!(revoke.actor_id, None, "the store wrote it with no actor");
    ids.extend(
        all.iter()
            .filter(|row| {
                row.workspace_id == Some(ws)
                    && matches!(
                        row.action.as_str(),
                        "scim.user.create" | "scim.user.delete" | "token.revoke"
                    )
            })
            .map(|row| row.id),
    );
    ids.sort_unstable();
    ids
}

async fn a_hold_keeps_only_its_own_workspaces_audit(db: &Db) {
    let store = db.store();
    let store = store.as_ref();
    let a = tenant(store, "held").await;
    let b = tenant(store, "free").await;

    let a_rows = actorless_rows(store, &a).await;
    let b_rows = actorless_rows(store, &b).await;
    assert_eq!(a_rows.len(), 5, "mint, attempt, create, delete, revoke");
    let instance = store
        .append_audit(actorless(
            AuditScope::Instance,
            "embeddings.reindex",
            "reindex_job",
        ))
        .await
        .expect("instance row");
    assert_eq!(instance.workspace_id, None);

    assert_eq!(ids_in_view(store, a.workspace_id).await, a_rows);
    assert_eq!(
        ids_in_view(store, b.workspace_id).await,
        b_rows,
        "B's view is exactly B's rows: none of A's, and no instance row"
    );

    store
        .place_legal_hold(a.workspace_id, "matter", None)
        .await
        .expect("hold A");
    db.backdate_audit(400).await;
    let pruned = store
        .prune_audit(Utc::now() - Duration::days(1), 1_000)
        .await
        .expect("prune");
    assert_eq!(
        pruned,
        (b_rows.len() + 1) as u64,
        "B's rows and the instance row prune while A is held"
    );
    assert_eq!(ids_in_view(store, a.workspace_id).await, a_rows, "A's kept");
    assert!(ids_in_view(store, b.workspace_id).await.is_empty());
    assert!(store
        .list_audit(500)
        .await
        .expect("all")
        .iter()
        .all(|row| row.workspace_id == Some(a.workspace_id)));
}

async fn the_backfill_derives_each_rows_workspace_from_what_it_references(db: &Db) {
    let store = db.store();
    let store = store.as_ref();
    let a = tenant(store, "bf-a").await;
    let b = tenant(store, "bf-b").await;
    let b_token = store
        .create_api_token(NewApiToken {
            workspace_id: b.workspace_id,
            member_id: b.id,
            app_installation_id: None,
            token_hash: hash_secret("bf-b-secret"),
            label: None,
            capabilities: vec![],
            expires_at: None,
        })
        .await
        .expect("token");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: b.workspace_id,
            name: "bf".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
        })
        .await
        .expect("thread");

    // Rows as they were before 0123: no workspace, and a view that could see
    // only the first and the sixth.
    let legacy = |action: &str, actor, kind: Option<&str>, target, metadata| NewAuditEvent {
        scope: AuditScope::Instance,
        actor_id: actor,
        action: action.into(),
        target_kind: kind.map(Into::into),
        target_id: target,
        metadata,
    };
    let none = serde_json::json!({});
    let expected = [
        (
            legacy(
                "workspace.target",
                None,
                Some("workspace"),
                Some(a.workspace_id.0),
                none.clone(),
            ),
            Some(a.workspace_id),
        ),
        (
            legacy(
                "metadata.workspace",
                None,
                Some("legal_hold"),
                Some(uuid::Uuid::now_v7()),
                serde_json::json!({ "workspace_id": b.workspace_id.0 }),
            ),
            Some(b.workspace_id),
        ),
        (
            legacy(
                "app_token.mint",
                None,
                Some("api_token"),
                Some(b_token.id.0),
                none.clone(),
            ),
            Some(b.workspace_id),
        ),
        (
            legacy(
                "scim.user.update",
                None,
                Some("member"),
                Some(a.id.0),
                none.clone(),
            ),
            Some(a.workspace_id),
        ),
        (
            legacy(
                "land_gate.clear",
                None,
                Some("thread"),
                Some(thread.id.0),
                none.clone(),
            ),
            Some(b.workspace_id),
        ),
        (
            legacy(
                "outbox.replay",
                Some(a.id),
                Some("outbox"),
                None,
                none.clone(),
            ),
            Some(a.workspace_id),
        ),
        (
            legacy(
                "embeddings.reindex",
                Some(a.id),
                Some("reindex_job"),
                Some(uuid::Uuid::now_v7()),
                serde_json::json!({ "workspace_id": null }),
            ),
            None,
        ),
        (
            legacy(
                "orphan",
                None,
                Some("member"),
                Some(uuid::Uuid::now_v7()),
                none.clone(),
            ),
            None,
        ),
    ];
    let mut ids = Vec::new();
    for (row, _) in &expected {
        ids.push(store.append_audit(row.clone()).await.expect("seed").id);
    }

    db.reapply_audit_workspace_migration().await;

    let rows = store.list_audit(500).await.expect("all");
    for (id, (row, want)) in ids.iter().zip(&expected) {
        let got = rows.iter().find(|r| r.id == *id).expect("row kept");
        assert_eq!(got.workspace_id, *want, "{}", row.action);
    }
}

async fn run_suite(db: Db) {
    a_hold_keeps_only_its_own_workspaces_audit(&db).await;
    the_backfill_derives_each_rows_workspace_from_what_it_references(&db).await;
}

#[tokio::test]
async fn audit_rows_belong_to_a_workspace_sqlite() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    run_suite(Db::Sqlite(pool)).await;
}

#[tokio::test]
async fn audit_rows_belong_to_a_workspace_postgres() {
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
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(Db::Postgres(pool)).await;
}
