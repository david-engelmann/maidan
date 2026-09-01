//! Installed app store + app token bearer resolution.

use std::time::Duration;

use maidan_auth::{hash_secret, resolve_bearer, TokenSecret};
use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn pool() -> Option<(testcontainers::ContainerAsync<Postgres>, sqlx::PgPool)> {
    let container = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
        .ok()?;
    let host = container.get_host().await.ok()?;
    let port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .ok()?;
    run_postgres_migrations(&pool).await.ok()?;
    Some((container, pool))
}

#[tokio::test]
async fn app_installation_token_resolves_and_subset_caps_enforced() {
    let Some((_c, pool)) = pool().await else {
        return;
    };
    let store = PostgresStore::new(pool);
    let ws = store
        .create_workspace(NewWorkspace {
            name: "app-store-ws".into(),
        })
        .await
        .unwrap();
    let app = store
        .create_app(NewApp {
            workspace_id: ws.id,
            slug: "my-bot".into(),
            name: "My Bot".into(),
            description: None,
            created_by: store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: "owner".into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
                .id,
        })
        .await
        .unwrap();
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "app:my-bot".into(),
            display_name: Some("My Bot".into()),
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let install = store
        .create_app_installation(NewAppInstallation {
            app_id: app.id,
            workspace_id: ws.id,
            bot_member_id: bot.id,
            granted_capabilities: vec!["message:post".into(), "workspace:read".into()],
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: bot.id,
            app_installation_id: Some(install.id),
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec!["message:post".into()],
            expires_at: None,
        })
        .await
        .unwrap();

    let ctx = resolve_bearer(&store, secret.as_str()).await.unwrap();
    assert_eq!(ctx.member_id, bot.id);
    assert_eq!(ctx.app_installation_id, Some(install.id));
    assert!(ctx.has_capability("message:post"));
    assert!(!ctx.has_capability("workspace:write"));

    store.revoke_app_installation(install.id).await.unwrap();
    assert!(resolve_bearer(&store, secret.as_str()).await.is_err());
}
