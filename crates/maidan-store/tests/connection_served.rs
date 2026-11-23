//! A workspace is served on a surface by a connection of its own or by the
//! operator's opening it. A workspace opened by sign-up is served only by its
//! own, and says to connect the surface when it has none. Both backends.

use std::time::Duration;

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    AuditScope, ConnectionSurface, EgressSurface, MemberId, NewAuditEvent, NewChannel,
    NewEgressTarget, NewGithubIssueLink, NewOidcSignup, NewSecret, NewSlackChannelLink, NewThread,
    NewWorkspace, WorkspaceId,
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

async fn signed_up(store: &dyn Store, who: &str) -> (WorkspaceId, MemberId) {
    let outcome = store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: chrono::Utc::now() + Duration::from_secs(3600),
                open: false,
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
        .expect("sign up");
    (outcome.workspace.id, outcome.member.id)
}

async fn connect(store: &dyn Store, ws: WorkspaceId, by: MemberId, surface: ConnectionSurface) {
    store
        .create_secret(NewSecret {
            workspace_id: ws,
            name: surface.secret_name().into(),
            value_ciphertext: "not a real ciphertext".into(),
            created_by: by,
        })
        .await
        .expect("store the connection");
}

fn bless(ws: WorkspaceId, surface: EgressSurface, selector: &str) -> NewEgressTarget {
    NewEgressTarget {
        workspace_id: ws,
        surface,
        selector: selector.into(),
        created_by: None,
    }
}

fn refused(result: Result<impl std::fmt::Debug, StoreError>, surface: ConnectionSurface) {
    match result {
        Err(StoreError::InvalidInput(message)) => assert_eq!(
            message,
            format!(
                "connect {} for this workspace first (Tools, Integrations)",
                surface.label()
            )
        ),
        other => panic!("not refused as unserved on {surface}: {other:?}"),
    }
}

async fn run_suite(store: &dyn Store) {
    let (ws, member) = signed_up(store, "casey").await;
    let (other, other_member) = signed_up(store, "devon").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: Some(member),
        })
        .await
        .expect("thread");

    // Nothing connected: nothing is served, and each write says which surface.
    for surface in ConnectionSurface::ALL {
        assert!(!store.workspace_served_on(ws, surface).await.unwrap());
    }
    refused(
        store
            .allow_egress_target(bless(ws, EgressSurface::Github, "example/repo"))
            .await,
        ConnectionSurface::Github,
    );
    refused(
        store
            .allow_egress_target(bless(ws, EgressSurface::GithubBranch, "example/repo@main"))
            .await,
        ConnectionSurface::Github,
    );
    refused(
        store
            .allow_egress_target(bless(ws, EgressSurface::Slack, "C0123ABCDEF"))
            .await,
        ConnectionSurface::Slack,
    );
    refused(
        store
            .link_github_issue(NewGithubIssueLink {
                repo: "example/repo".into(),
                issue_number: 1,
                workspace_id: ws,
                channel_id: channel.id,
                thread_id: thread.id,
                member_id: member,
            })
            .await,
        ConnectionSurface::Github,
    );
    refused(
        store
            .link_slack_channel(NewSlackChannelLink {
                slack_channel_id: "C0123ABCDEF".into(),
                workspace_id: ws,
                channel_id: channel.id,
                thread_id: thread.id,
                member_id: member,
            })
            .await,
        ConnectionSurface::Slack,
    );
    refused(
        maidan_store::pull_watch::repo_blessed(store, ws, "example/repo").await,
        ConnectionSurface::Github,
    );

    // A GitHub connection serves GitHub and its branch surface, not Slack.
    connect(store, ws, member, ConnectionSurface::Github).await;
    assert!(store
        .workspace_served_on(ws, ConnectionSurface::Github)
        .await
        .unwrap());
    assert!(!store
        .workspace_served_on(ws, ConnectionSurface::Slack)
        .await
        .unwrap());
    store
        .allow_egress_target(bless(ws, EgressSurface::Github, "example/repo"))
        .await
        .expect("bless github");
    store
        .allow_egress_target(bless(ws, EgressSurface::GithubBranch, "example/repo@main"))
        .await
        .expect("bless the branch surface with the github connection");
    refused(
        store
            .allow_egress_target(bless(ws, EgressSurface::Slack, "C0123ABCDEF"))
            .await,
        ConnectionSurface::Slack,
    );
    store
        .link_github_issue(NewGithubIssueLink {
            repo: "example/repo".into(),
            issue_number: 1,
            workspace_id: ws,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: member,
        })
        .await
        .expect("link an issue");
    assert!(
        maidan_store::pull_watch::repo_blessed(store, ws, "example/repo")
            .await
            .unwrap()
    );
    assert!(store
        .is_egress_target_allowed(ws, EgressSurface::Github, "example/repo")
        .await
        .unwrap());

    // A connection belongs to its workspace alone.
    assert!(!store
        .workspace_served_on(other, ConnectionSurface::Github)
        .await
        .unwrap());
    refused(
        store
            .allow_egress_target(bless(other, EgressSurface::Github, "example/repo"))
            .await,
        ConnectionSurface::Github,
    );
    connect(store, other, other_member, ConnectionSurface::Slack).await;
    assert!(!store
        .workspace_served_on(other, ConnectionSurface::Github)
        .await
        .unwrap());

    // Removing the connection ends service, and what was blessed counts for
    // nothing until one returns.
    store
        .delete_secret(ws, ConnectionSurface::Github.secret_name())
        .await
        .expect("disconnect");
    assert!(!store
        .workspace_served_on(ws, ConnectionSurface::Github)
        .await
        .unwrap());
    assert!(!store
        .is_egress_target_allowed(ws, EgressSurface::Github, "example/repo")
        .await
        .unwrap());
    assert!(!store
        .is_egress_target_allowed(ws, EgressSurface::GithubBranch, "example/repo@main")
        .await
        .unwrap());

    // The operator's workspace is served with no connection of its own.
    let ops = store
        .create_workspace(NewWorkspace { name: "ops".into() })
        .await
        .expect("operator workspace");
    for surface in ConnectionSurface::ALL {
        assert!(store.workspace_served_on(ops.id, surface).await.unwrap());
    }
    store
        .allow_egress_target(bless(
            ops.id,
            EgressSurface::Linear,
            "4a3f6b7e-0c1d-4e2f-9a8b-1c2d3e4f5a6b",
        ))
        .await
        .expect("the operator's workspace blesses without a connection");

    // A workspace that is not there is not found, not refused.
    assert!(matches!(
        store
            .workspace_served_on(WorkspaceId::new(), ConnectionSurface::Github)
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn a_workspace_is_served_by_its_own_connection_or_the_operators_opening_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn a_workspace_is_served_by_its_own_connection_or_the_operators_opening_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
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
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
