//! Workspace-scoped installed apps and installations (Cluster 57.0).

use chrono::{DateTime, Utc};
use maidan_types::{
    App, AppId, AppInstallation, AppInstallationId, MemberId, NewApp, NewAppInstallation,
    WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create_app(pool: &PgPool, new: NewApp) -> Result<App, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_apps (id, workspace_id, slug, name, description, created_by)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, workspace_id, slug, name, description, created_by, created_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.slug)
    .bind(&new.name)
    .bind(new.description.as_deref())
    .bind(new.created_by.0)
    .fetch_one(pool)
    .await
    .map_err(map_app_err)?;
    row_to_app(&row)
}

pub async fn get_app(pool: &PgPool, id: AppId) -> Result<App, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, slug, name, description, created_by, created_at
         FROM maidan_apps WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_app(&row)
}

pub async fn list_apps(pool: &PgPool, workspace_id: WorkspaceId) -> Result<Vec<App>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, slug, name, description, created_by, created_at
         FROM maidan_apps
         WHERE workspace_id = $1
         ORDER BY slug ASC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_app).collect()
}

pub async fn create_installation(
    pool: &PgPool,
    new: NewAppInstallation,
) -> Result<AppInstallation, StoreError> {
    let id = Uuid::new_v4();
    let caps = serde_json::to_string(&new.granted_capabilities)?;
    let row = sqlx::query(
        "INSERT INTO maidan_app_installations
            (id, app_id, workspace_id, bot_member_id, granted_capabilities)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, app_id, workspace_id, bot_member_id, granted_capabilities,
                   installed_at, revoked_at",
    )
    .bind(id)
    .bind(new.app_id.0)
    .bind(new.workspace_id.0)
    .bind(new.bot_member_id.0)
    .bind(&caps)
    .fetch_one(pool)
    .await?;
    row_to_installation(&row)
}

pub async fn get_installation(
    pool: &PgPool,
    id: AppInstallationId,
) -> Result<AppInstallation, StoreError> {
    let row = sqlx::query(
        "SELECT id, app_id, workspace_id, bot_member_id, granted_capabilities,
                installed_at, revoked_at
         FROM maidan_app_installations WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_installation(&row)
}

pub async fn list_installations(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<AppInstallation>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, app_id, workspace_id, bot_member_id, granted_capabilities,
                installed_at, revoked_at
         FROM maidan_app_installations
         WHERE workspace_id = $1
         ORDER BY installed_at DESC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_installation).collect()
}

pub async fn revoke_installation(
    pool: &PgPool,
    id: AppInstallationId,
) -> Result<AppInstallation, StoreError> {
    let now = Utc::now();
    let row = sqlx::query(
        "UPDATE maidan_app_installations
         SET revoked_at = $2
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING id, app_id, workspace_id, bot_member_id, granted_capabilities,
                   installed_at, revoked_at",
    )
    .bind(id.0)
    .bind(now)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    sqlx::query(
        "UPDATE maidan_api_tokens SET revoked_at = $2
         WHERE app_installation_id = $1 AND revoked_at IS NULL",
    )
    .bind(id.0)
    .bind(now)
    .execute(pool)
    .await?;
    row_to_installation(&row)
}

fn map_app_err(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db) = err {
        if db.is_unique_violation() {
            return StoreError::Conflict("app slug already exists in workspace".into());
        }
    }
    StoreError::Database(err)
}

fn parse_caps(json: &str) -> Result<Vec<String>, StoreError> {
    serde_json::from_str(json)
        .map_err(|e| StoreError::InvalidInput(format!("invalid capabilities JSON: {e}")))
}

fn row_to_app(row: &sqlx::postgres::PgRow) -> Result<App, StoreError> {
    Ok(App {
        id: AppId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        slug: row.get("slug"),
        name: row.get("name"),
        description: row.get("description"),
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

fn row_to_installation(row: &sqlx::postgres::PgRow) -> Result<AppInstallation, StoreError> {
    let caps: String = row.get("granted_capabilities");
    Ok(AppInstallation {
        id: AppInstallationId(row.get::<Uuid, _>("id")),
        app_id: AppId(row.get::<Uuid, _>("app_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        bot_member_id: MemberId(row.get::<Uuid, _>("bot_member_id")),
        granted_capabilities: parse_caps(&caps)?,
        installed_at: row.get::<DateTime<Utc>, _>("installed_at"),
        revoked_at: row.get::<Option<DateTime<Utc>>, _>("revoked_at"),
    })
}
