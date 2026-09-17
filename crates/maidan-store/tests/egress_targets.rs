//! The egress trust boundary: a per-workspace allowlist of the destinations
//! Maidan may deliver to. Bless (idempotent, selector-validated) / list /
//! revoke (workspace-scoped) / the authorization check itself. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    EgressSurface, EgressTarget, EgressTargetId, NewEgressTarget, NewWorkspace, WorkspaceId,
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
    SqliteStore::new(pool)
}

fn target(ws: WorkspaceId, surface: EgressSurface, selector: &str) -> NewEgressTarget {
    NewEgressTarget {
        workspace_id: ws,
        surface,
        selector: selector.into(),
    }
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "egress-allow".into(),
        })
        .await
        .expect("ws");
    let other = store
        .create_workspace(NewWorkspace {
            name: "egress-allow-other".into(),
        })
        .await
        .expect("other ws");

    // Nothing configured means nothing is trusted — the fail-safe default.
    assert!(store
        .list_egress_targets(ws.id)
        .await
        .expect("list empty")
        .is_empty());
    assert!(
        !store
            .is_egress_target_allowed(ws.id, EgressSurface::Slack, "C0123ABCDEF")
            .await
            .expect("deny by default"),
        "an empty allowlist authorizes nothing"
    );

    let slack = store
        .allow_egress_target(target(ws.id, EgressSurface::Slack, "C0123ABCDEF"))
        .await
        .expect("bless slack");
    assert_eq!(slack.workspace_id, ws.id);
    assert_eq!(slack.surface, "slack");
    assert_eq!(slack.selector, "C0123ABCDEF");
    let github = store
        .allow_egress_target(target(ws.id, EgressSurface::Github, "example/repo"))
        .await
        .expect("bless github");

    assert!(store
        .is_egress_target_allowed(ws.id, EgressSurface::Slack, "C0123ABCDEF")
        .await
        .expect("slack allowed"));
    assert!(store
        .is_egress_target_allowed(ws.id, EgressSurface::Github, "example/repo")
        .await
        .expect("github allowed"));

    // The check is keyed on all three columns: a near miss is not a hit.
    for (surface, selector) in [
        (EgressSurface::Slack, "C0000000000"),
        (EgressSurface::Github, "example/other"),
        // Same selector text, wrong surface.
        (EgressSurface::Github, "C0123ABCDEF"),
        (EgressSurface::Slack, "example/repo"),
    ] {
        assert!(
            !store
                .is_egress_target_allowed(ws.id, surface, selector)
                .await
                .expect("near miss"),
            "{surface}:{selector} must not be authorized"
        );
    }

    // A blessing is per workspace. One tenant's allowlist never authorizes
    // another's delivery, even for the identical destination.
    assert!(
        !store
            .is_egress_target_allowed(other.id, EgressSurface::Slack, "C0123ABCDEF")
            .await
            .expect("other ws"),
        "a blessing does not leak across workspaces"
    );

    // The authorization grain: a GitHub *delivery* is `owner/name#123`, but the
    // blessing is the repository — so the repo blessing authorizes any issue in
    // it, and the delivery selector itself is not what gets looked up.
    let delivery = EgressTarget::Github {
        repo: "example/repo".into(),
        issue_number: 3915,
    };
    assert!(store
        .is_egress_target_allowed(ws.id, delivery.surface(), &delivery.allowlist_selector())
        .await
        .expect("repo blessing covers the issue"));
    assert!(
        !store
            .is_egress_target_allowed(ws.id, delivery.surface(), &delivery.selector())
            .await
            .expect("delivery selector is not the allowlist key"),
        "the issue-qualified selector is deliberately not an allowlist key"
    );

    // Re-blessing is idempotent: the same entry, not a duplicate row.
    let again = store
        .allow_egress_target(target(ws.id, EgressSurface::Slack, "C0123ABCDEF"))
        .await
        .expect("re-bless");
    assert_eq!(again.id, slack.id, "a re-bless keeps the original entry");
    assert_eq!(again.created_at, slack.created_at);

    let listed = store.list_egress_targets(ws.id).await.expect("list");
    assert_eq!(listed.len(), 2, "the re-bless did not add a row");
    assert_eq!(
        listed
            .iter()
            .map(|t| t.surface.as_str())
            .collect::<Vec<_>>(),
        vec!["github", "slack"],
        "ordered by surface then selector"
    );

    // A selector that is a *name* rather than an id is refused in the store, so
    // every write path inherits the rule.
    for (surface, selector) in [
        (EgressSurface::Slack, "#general"),
        (EgressSurface::Github, "example/repo#42"),
        (EgressSurface::Github, "widgets"),
    ] {
        let err = store
            .allow_egress_target(target(ws.id, surface, selector))
            .await
            .expect_err("must be refused");
        assert!(
            matches!(err, StoreError::InvalidInput(_)),
            "{surface}:{selector} should be InvalidInput, got {err:?}"
        );
    }

    // Revoking is workspace-scoped: the other tenant cannot revoke this entry by
    // id, and after a real revoke the authorization goes away.
    assert!(
        !store
            .revoke_egress_target(other.id, slack.id)
            .await
            .expect("cross-tenant revoke"),
        "an id from another workspace does not revoke"
    );
    assert!(store
        .is_egress_target_allowed(ws.id, EgressSurface::Slack, "C0123ABCDEF")
        .await
        .expect("still allowed"));

    assert!(store
        .revoke_egress_target(ws.id, slack.id)
        .await
        .expect("revoke"));
    assert!(!store
        .is_egress_target_allowed(ws.id, EgressSurface::Slack, "C0123ABCDEF")
        .await
        .expect("revoked"));
    assert!(
        !store
            .revoke_egress_target(ws.id, slack.id)
            .await
            .expect("revoke twice"),
        "a second revoke reports nothing removed"
    );
    assert!(
        !store
            .revoke_egress_target(ws.id, EgressTargetId::new())
            .await
            .expect("unknown id"),
        "an unknown id reports nothing removed"
    );

    let remaining = store.list_egress_targets(ws.id).await.expect("list after");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, github.id);
}

#[tokio::test]
async fn egress_targets_allow_list_revoke_and_authorize_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn egress_targets_allow_list_revoke_and_authorize_postgres() {
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
            eprintln!("skipping: docker unavailable ({err})");
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
