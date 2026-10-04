//! Workspace-scoped installed apps and installations.

use chrono::{DateTime, Utc};
use maidan_types::{
    App, AppId, AppInstallation, AppInstallationId, MemberId, MemberKind, NewApp,
    NewAppInstallation, NewMember, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::members;
use crate::error::StoreError;
use crate::InstalledApp;

pub async fn create_app(pool: &PgPool, new: NewApp) -> Result<App, StoreError> {
    let id = Uuid::now_v7();
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
    let mut conn = pool.acquire().await?;
    insert_installation_on(&mut conn, new).await
}

async fn insert_installation_on(
    conn: &mut sqlx::PgConnection,
    new: NewAppInstallation,
) -> Result<AppInstallation, StoreError> {
    let id = Uuid::now_v7();
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
    .fetch_one(&mut *conn)
    .await?;
    row_to_installation(&row)
}

/// Install `app_id` in `workspace_id` on `conn` (the caller's transaction).
/// The bot member is the one the app's latest revoked installation in this
/// workspace used, when there is one and it is not tombstoned; otherwise a new
/// `app:<slug>` agent member, which a hand-made member holding that handle
/// refuses rather than being taken over.
pub(crate) async fn install_on(
    conn: &mut sqlx::PgConnection,
    workspace_id: WorkspaceId,
    app_id: AppId,
    granted_capabilities: &[String],
) -> Result<InstalledApp, StoreError> {
    // The app row's lock serializes installs of one app, so two cannot both
    // find no active installation and both proceed.
    let app = sqlx::query(
        "SELECT slug, name FROM maidan_apps WHERE id = $1 AND workspace_id = $2
         FOR UPDATE",
    )
    .bind(app_id.0)
    .bind(workspace_id.0)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(StoreError::NotFound)?;
    let slug: String = app.get("slug");
    let name: String = app.get("name");

    let active = sqlx::query(
        "SELECT 1 FROM maidan_app_installations
         WHERE app_id = $1 AND workspace_id = $2 AND revoked_at IS NULL",
    )
    .bind(app_id.0)
    .bind(workspace_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    if active.is_some() {
        return Err(StoreError::Conflict(
            "app is already installed in this workspace; revoke the installation to change its grants".into(),
        ));
    }

    let previous: Option<Uuid> = sqlx::query_scalar(
        "SELECT i.bot_member_id
         FROM maidan_app_installations i
         JOIN maidan_members m ON m.id = i.bot_member_id
         WHERE i.app_id = $1 AND i.workspace_id = $2
           AND m.workspace_id = i.workspace_id AND m.tombstoned_at IS NULL
         ORDER BY i.installed_at DESC, i.id DESC
         LIMIT 1",
    )
    .bind(app_id.0)
    .bind(workspace_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    let (bot_member_id, bot_member_reused) = match previous {
        Some(id) => (MemberId(id), true),
        None => {
            let bot = members::create_on(
                &mut *conn,
                NewMember {
                    workspace_id,
                    handle: format!("app:{slug}"),
                    display_name: Some(name),
                    kind: MemberKind::Agent,
                },
            )
            .await?;
            (bot.id, false)
        }
    };

    let installation = insert_installation_on(
        conn,
        NewAppInstallation {
            app_id,
            workspace_id,
            bot_member_id,
            granted_capabilities: granted_capabilities.to_vec(),
        },
    )
    .await?;
    Ok(InstalledApp {
        installation,
        bot_member_reused,
    })
}

pub async fn get_installation(
    pool: &PgPool,
    id: AppInstallationId,
) -> Result<AppInstallation, StoreError> {
    let mut conn = pool.acquire().await?;
    get_installation_on(&mut conn, id).await
}

pub(crate) async fn get_installation_on(
    conn: &mut sqlx::PgConnection,
    id: AppInstallationId,
) -> Result<AppInstallation, StoreError> {
    let row = sqlx::query(
        "SELECT id, app_id, workspace_id, bot_member_id, granted_capabilities,
                installed_at, revoked_at
         FROM maidan_app_installations WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(&mut *conn)
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
    let mut conn = pool.acquire().await?;
    revoke_installation_on(&mut conn, id).await
}

pub(crate) async fn revoke_installation_on(
    conn: &mut sqlx::PgConnection,
    id: AppInstallationId,
) -> Result<AppInstallation, StoreError> {
    // The installation and every token minted under it are revoked together.
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
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
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    sqlx::query(
        "UPDATE maidan_api_tokens SET revoked_at = $2
         WHERE app_installation_id = $1 AND revoked_at IS NULL",
    )
    .bind(id.0)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
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
