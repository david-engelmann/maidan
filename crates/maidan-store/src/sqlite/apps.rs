//! Workspace-scoped installed apps (SQLite).

use chrono::{DateTime, Utc};
use maidan_types::{
    App, AppId, AppInstallation, AppInstallationId, MemberId, NewApp, NewAppInstallation,
    WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn parse_ts(s: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| StoreError::InvalidInput(format!("bad timestamp: {e}")))
}

pub async fn create_app(pool: &SqlitePool, new: NewApp) -> Result<App, StoreError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO maidan_apps (id, workspace_id, slug, name, description, created_by)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.slug)
    .bind(&new.name)
    .bind(new.description.as_deref())
    .bind(new.created_by.0)
    .execute(pool)
    .await
    .map_err(map_app_err)?;
    get_app(pool, AppId(id)).await
}

pub async fn get_app(pool: &SqlitePool, id: AppId) -> Result<App, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, slug, name, description, created_by, created_at
         FROM maidan_apps WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_app(&row)
}

pub async fn list_apps(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<App>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, slug, name, description, created_by, created_at
         FROM maidan_apps
         WHERE workspace_id = ?
         ORDER BY slug ASC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_app).collect()
}

pub async fn create_installation(
    pool: &SqlitePool,
    new: NewAppInstallation,
) -> Result<AppInstallation, StoreError> {
    let id = Uuid::new_v4();
    let caps = serde_json::to_string(&new.granted_capabilities)?;
    sqlx::query(
        "INSERT INTO maidan_app_installations
            (id, app_id, workspace_id, bot_member_id, granted_capabilities)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(new.app_id.0)
    .bind(new.workspace_id.0)
    .bind(new.bot_member_id.0)
    .bind(&caps)
    .execute(pool)
    .await?;
    get_installation(pool, AppInstallationId(id)).await
}

pub async fn get_installation(
    pool: &SqlitePool,
    id: AppInstallationId,
) -> Result<AppInstallation, StoreError> {
    let row = sqlx::query(
        "SELECT id, app_id, workspace_id, bot_member_id, granted_capabilities,
                installed_at, revoked_at
         FROM maidan_app_installations WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_installation(&row)
}

pub async fn list_installations(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<AppInstallation>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, app_id, workspace_id, bot_member_id, granted_capabilities,
                installed_at, revoked_at
         FROM maidan_app_installations
         WHERE workspace_id = ?
         ORDER BY installed_at DESC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_installation).collect()
}

pub async fn revoke_installation(
    pool: &SqlitePool,
    id: AppInstallationId,
) -> Result<AppInstallation, StoreError> {
    let now = Utc::now().to_rfc3339();
    let updated = sqlx::query(
        "UPDATE maidan_app_installations
         SET revoked_at = ?
         WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(&now)
    .bind(id.0)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    sqlx::query(
        "UPDATE maidan_api_tokens SET revoked_at = ?
         WHERE app_installation_id = ? AND revoked_at IS NULL",
    )
    .bind(&now)
    .bind(id.0)
    .execute(pool)
    .await?;
    get_installation(pool, id).await
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

fn row_to_app(row: &sqlx::sqlite::SqliteRow) -> Result<App, StoreError> {
    let created_at: String = row.get("created_at");
    Ok(App {
        id: AppId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        slug: row.get("slug"),
        name: row.get("name"),
        description: row.get("description"),
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: parse_ts(&created_at)?,
    })
}

fn row_to_installation(row: &sqlx::sqlite::SqliteRow) -> Result<AppInstallation, StoreError> {
    let caps: String = row.get("granted_capabilities");
    let installed_at: String = row.get("installed_at");
    let revoked_at: Option<String> = row.get("revoked_at");
    Ok(AppInstallation {
        id: AppInstallationId(row.get::<Uuid, _>("id")),
        app_id: AppId(row.get::<Uuid, _>("app_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        bot_member_id: MemberId(row.get::<Uuid, _>("bot_member_id")),
        granted_capabilities: parse_caps(&caps)?,
        installed_at: parse_ts(&installed_at)?,
        revoked_at: revoked_at.as_deref().map(parse_ts).transpose()?,
    })
}
