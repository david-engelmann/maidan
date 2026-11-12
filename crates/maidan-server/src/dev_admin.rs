//! A known first-admin token for a development stack (R11).
//!
//! The server always authenticates, so a compose stack, a Helm CI install or a
//! smoke script needs a real token before it can do anything. `maidan init`
//! mints one into a database it can open; a stack whose database sits in
//! another container sets `MAIDAN_DEV_ADMIN_TOKEN` instead. On an instance
//! with no workspace yet, boot then opens the first workspace (`dev`) with an
//! admin holding every capability, whose bearer is that value. It is a real
//! token: hashed, audited, revocable, and checked like any other. Refused under
//! `MAIDAN_ENV=production` and when shorter than [`MIN_LEN`].

use maidan_auth::{capability, hash_secret, resolve_bearer};
use maidan_store::Store;
use maidan_types::{
    AuditScope, MemberKind, NewApiToken, NewAuditEvent, NewMember, NewWorkspace, WorkspaceId,
};

use crate::config::ConfigError;

pub const ENV: &str = "MAIDAN_DEV_ADMIN_TOKEN";

/// The shortest secret accepted: a token anyone could guess is not one.
pub const MIN_LEN: usize = 32;

/// The first workspace's name when this variable opens it.
pub const WORKSPACE_NAME: &str = "dev";

/// Whether `secret` may be used here. Pure over its inputs so it is tested
/// without touching the process environment.
pub(crate) fn validate(production: bool, secret: &str) -> Result<(), ConfigError> {
    if production {
        return Err(ConfigError::Invalid(
            ENV,
            "cannot be set when MAIDAN_ENV=production; mint the first admin with `maidan init`"
                .into(),
        ));
    }
    if secret.trim() != secret || secret.chars().count() < MIN_LEN {
        return Err(ConfigError::Invalid(
            ENV,
            format!("must be at least {MIN_LEN} characters with no surrounding whitespace"),
        ));
    }
    Ok(())
}

/// What the variable did at this boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevAdmin {
    /// It opened the first workspace with this admin token.
    Seeded(WorkspaceId),
    /// The token already resolves, from an earlier boot or another replica.
    Present(WorkspaceId),
    /// The instance already has workspaces and none holds this token, so
    /// nothing was made: the variable only ever opens the first workspace.
    Skipped,
}

/// Apply `MAIDAN_DEV_ADMIN_TOKEN`, or do nothing when it is unset.
pub async fn seed_from_env(store: &dyn Store) -> Result<Option<DevAdmin>, ConfigError> {
    let secret = match std::env::var(ENV) {
        Ok(secret) => secret,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(ConfigError::Invalid(ENV, "is not valid Unicode".into()));
        }
    };
    validate(crate::config::is_production(), &secret)?;
    seed(store, &secret).await.map(Some)
}

/// Open the first workspace with `secret` as its admin's bearer, unless the
/// token already resolves or another workspace exists.
pub async fn seed(store: &dyn Store, secret: &str) -> Result<DevAdmin, ConfigError> {
    let failed = |what: &str, err: &dyn std::fmt::Display| {
        ConfigError::Invalid(ENV, format!("{what}: {err}"))
    };
    if let Ok(auth) = resolve_bearer(store, secret).await {
        return Ok(DevAdmin::Present(auth.workspace_id()));
    }
    let existing = store
        .count_workspaces()
        .await
        .map_err(|e| failed("count workspaces", &e))?;
    if existing > 0 {
        return Ok(DevAdmin::Skipped);
    }
    let (workspace, _) = store
        .create_workspace_with_event(NewWorkspace {
            name: WORKSPACE_NAME.into(),
        })
        .await
        .map_err(|e| failed("create workspace", &e))?;
    let (member, _) = store
        .create_member_with_event(NewMember {
            workspace_id: workspace.id,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .map_err(|e| failed("create admin member", &e))?;
    let actor = member.id;
    store
        .create_api_token_audited(
            NewApiToken {
                workspace_id: workspace.id,
                member_id: member.id,
                app_installation_id: None,
                token_hash: hash_secret(secret),
                label: Some(ENV.into()),
                capabilities: capability::all(),
                expires_at: None,
            },
            Box::new(move |record| NewAuditEvent {
                scope: AuditScope::Workspace(record.workspace_id),
                actor_id: Some(actor),
                action: "token.mint".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(record.id.0),
                metadata: serde_json::json!({
                    "workspace_id": record.workspace_id.0,
                    "subject_member_id": record.member_id.0,
                    "capabilities": record.capabilities.clone(),
                    "source": ENV,
                }),
            }),
        )
        .await
        .map_err(|e| failed("mint admin token", &e))?;
    Ok(DevAdmin::Seeded(workspace.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "dev-admin-token-for-tests-0123456789abcdef";

    #[test]
    fn the_token_is_refused_in_production_short_or_padded() {
        assert!(validate(true, SECRET).is_err());
        assert!(validate(false, "short").is_err());
        assert!(validate(false, &format!(" {SECRET}")).is_err());
        assert!(validate(false, SECRET).is_ok());
    }

    async fn store() -> std::sync::Arc<dyn Store> {
        let pool = maidan_search::sqlite_pool_options()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        maidan_store::run_sqlite_migrations(&pool).await.unwrap();
        std::sync::Arc::new(maidan_store::SqliteStore::for_tests(pool))
    }

    #[tokio::test]
    async fn an_empty_instance_gets_a_real_admin_token_once() {
        let store = store().await;
        let DevAdmin::Seeded(workspace) = seed(store.as_ref(), SECRET).await.unwrap() else {
            panic!("an empty instance is seeded");
        };
        let auth = resolve_bearer(store.as_ref(), SECRET).await.unwrap();
        assert_eq!(auth.workspace_id(), workspace);
        assert!(auth.has_capability(capability::OPERATOR_GLOBAL));
        assert_eq!(
            seed(store.as_ref(), SECRET).await.unwrap(),
            DevAdmin::Present(workspace)
        );
        assert_eq!(store.count_workspaces().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn an_instance_with_a_workspace_is_left_alone() {
        let store = store().await;
        store
            .create_workspace(NewWorkspace {
                name: "real".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            seed(store.as_ref(), SECRET).await.unwrap(),
            DevAdmin::Skipped
        );
        assert!(resolve_bearer(store.as_ref(), SECRET).await.is_err());
    }
}
