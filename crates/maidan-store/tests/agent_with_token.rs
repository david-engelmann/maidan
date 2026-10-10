//! `create_agent_with_token`: an agent member, its first token and both audit
//! rows commit together or not at all. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApiToken, AuditScope, Member, MemberKind, NewAuditEvent, NewMember, NewWorkspace,
};
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
    SqliteStore::for_tests(pool)
}

fn new_agent(ws: maidan_types::WorkspaceId, handle: &str, hash: &str) -> NewAgentWithToken {
    NewAgentWithToken {
        workspace_id: ws,
        handle: handle.into(),
        display_name: Some("Builder".into()),
        token_hash: hash.into(),
        token_label: Some(handle.into()),
        capabilities: vec!["workspace:read".into(), "message:write".into()],
        expires_at: None,
    }
}

fn audits(
    actor: maidan_types::MemberId,
) -> (
    maidan_store::AuditFor<Member>,
    maidan_store::AuditFor<ApiToken>,
) {
    (
        Box::new(move |m: &Member| NewAuditEvent {
            scope: AuditScope::Workspace(m.workspace_id),
            actor_id: Some(actor),
            action: "member.create".into(),
            target_kind: Some("member".into()),
            target_id: Some(m.id.0),
            metadata: serde_json::json!({}),
        }),
        Box::new(move |t: &ApiToken| NewAuditEvent {
            scope: AuditScope::Workspace(t.workspace_id),
            actor_id: Some(actor),
            action: "token.mint".into(),
            target_kind: Some("api_token".into()),
            target_id: Some(t.id.0),
            metadata: serde_json::json!({}),
        }),
    )
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let admin = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "root".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("admin");
    let hash_a = "a".repeat(64);

    // The member is an agent, the token is its own, and both audit rows land.
    let (ma, ta) = audits(admin.id);
    let created = store
        .create_agent_with_token(new_agent(ws.id, "builder", &hash_a), ma, ta)
        .await
        .expect("create");
    assert_eq!(created.member.kind, MemberKind::Agent);
    assert_eq!(created.member.handle, "builder");
    assert_eq!(created.member.display_name.as_deref(), Some("Builder"));
    assert_eq!(created.token.member_id, created.member.id);
    assert_eq!(created.token.workspace_id, ws.id);
    assert_eq!(
        created.token.capabilities,
        vec!["workspace:read".to_string(), "message:write".to_string()]
    );
    assert_eq!(
        store
            .get_active_api_token_by_hash(&hash_a)
            .await
            .expect("lookup")
            .id,
        created.token.id
    );
    let rows = store
        .list_audit_for_workspace(ws.id, 50)
        .await
        .expect("audit");
    let member_row = rows
        .iter()
        .find(|r| r.action == "member.create")
        .expect("member.create row");
    assert_eq!(member_row.target_id, Some(created.member.id.0));
    assert_eq!(member_row.actor_id, Some(admin.id));
    let token_row = rows
        .iter()
        .find(|r| r.action == "token.mint")
        .expect("token.mint row");
    assert_eq!(token_row.target_id, Some(created.token.id.0));

    // A taken handle refuses before anything is written.
    let (ma, ta) = audits(admin.id);
    let err = store
        .create_agent_with_token(new_agent(ws.id, "builder", &"b".repeat(64)), ma, ta)
        .await
        .err()
        .expect("duplicate handle refused");
    assert!(matches!(err, StoreError::Conflict(_)), "{err:?}");
    assert!(matches!(
        store.get_active_api_token_by_hash(&"b".repeat(64)).await,
        Err(StoreError::NotFound)
    ));

    // A token that cannot be written takes its member with it: same hash as
    // the first token, so the token insert fails after the member insert.
    let (ma, ta) = audits(admin.id);
    store
        .create_agent_with_token(new_agent(ws.id, "orphan", &hash_a), ma, ta)
        .await
        .err()
        .expect("token insert refused");
    assert!(
        matches!(
            store.get_member_by_handle(ws.id, "orphan").await,
            Err(StoreError::NotFound)
        ),
        "the member must roll back with its token"
    );
    let after = store
        .list_audit_for_workspace(ws.id, 50)
        .await
        .expect("audit");
    assert_eq!(
        after.iter().filter(|r| r.action == "member.create").count(),
        1,
        "no audit row for a member that was never created"
    );
}

#[tokio::test]
async fn agent_with_token_commits_together_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn agent_with_token_commits_together_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
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
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
