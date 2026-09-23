use chrono::{DateTime, Utc};
use maidan_types::{ApiToken, ApiTokenId, AppInstallationId, MemberId, NewApiToken, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &PgPool, new: NewApiToken) -> Result<ApiToken, StoreError> {
    let id = Uuid::new_v4();
    let capabilities = serde_json::to_string(&new.capabilities)?;
    let row = sqlx::query(
        "INSERT INTO maidan_api_tokens
            (id, workspace_id, member_id, app_installation_id, token_hash, label, capabilities, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, workspace_id, member_id, app_installation_id, token_hash, label, capabilities,
                   created_at, expires_at, revoked_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(new.app_installation_id.map(|i| i.0))
    .bind(&new.token_hash)
    .bind(new.label.as_deref())
    .bind(&capabilities)
    .bind(new.expires_at)
    .fetch_one(pool)
    .await
    .map_err(map_token_err)?;
    row_to_token(&row)
}

/// Mint a token that records the token it was derived from.
///
/// Separate from [`create`] rather than a field on `NewApiToken`: that struct
/// is built at 109 sites, 100 of them tests, and only the attenuation path has
/// a parent. Adding a field there would have been a hundred mechanical edits
/// for one caller.
pub async fn create_attenuated(
    pool: &PgPool,
    new: NewApiToken,
    parent_token_id: ApiTokenId,
) -> Result<ApiToken, StoreError> {
    let id = Uuid::new_v4();
    let capabilities = serde_json::to_string(&new.capabilities)?;
    let row = sqlx::query(
        "INSERT INTO maidan_api_tokens
            (id, workspace_id, member_id, app_installation_id, token_hash, label, capabilities, expires_at, parent_token_id, delegation_grant_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 (SELECT delegation_grant_id FROM maidan_api_tokens WHERE id = $9))
         RETURNING id, workspace_id, member_id, app_installation_id, token_hash, label, capabilities,
                   created_at, expires_at, revoked_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(new.app_installation_id.map(|i| i.0))
    .bind(&new.token_hash)
    .bind(new.label.as_deref())
    .bind(&capabilities)
    .bind(new.expires_at)
    .bind(parent_token_id.0)
    .fetch_one(pool)
    .await
    .map_err(map_token_err)?;
    row_to_token(&row)
}

pub async fn create_delegated(
    pool: &PgPool,
    new: NewApiToken,
    grant_id: maidan_types::DelegationGrantId,
    delegate_id: MemberId,
    parent_token_id: Option<ApiTokenId>,
) -> Result<ApiToken, StoreError> {
    let grant_capabilities_json: String = sqlx::query_scalar(
        "SELECT capabilities FROM maidan_delegation_grants
         WHERE id=$1 AND workspace_id=$2 AND subject_id=$3 AND delegate_id=$4",
    )
    .bind(grant_id.0)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(delegate_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    let grant_capabilities: Vec<String> = serde_json::from_str(&grant_capabilities_json)?;
    let expires_at =
        crate::delegation_grants::validate_exchange(&new, &grant_capabilities, Utc::now())?;
    let id = Uuid::new_v4();
    let capabilities = serde_json::to_string(&new.capabilities)?;
    let row = sqlx::query(
        "INSERT INTO maidan_api_tokens
            (id, workspace_id, member_id, app_installation_id, token_hash, label,
             capabilities, expires_at, parent_token_id, delegation_grant_id)
         SELECT $1, $2, $3, NULL, $4, $5, $6, $7, $8, g.id
         FROM maidan_delegation_grants g
         WHERE g.id = $9 AND g.workspace_id = $2 AND g.subject_id = $3
           AND g.delegate_id = $10 AND g.revoked_at IS NULL
           AND g.expires_at >= $7
           AND ($8 IS NULL OR EXISTS (
               SELECT 1 FROM maidan_api_tokens p
               WHERE p.id = $8 AND p.workspace_id = $2 AND p.member_id = $10
                 AND p.revoked_at IS NULL
                 AND (p.expires_at IS NULL OR p.expires_at >= $7)))
         RETURNING id, workspace_id, member_id, app_installation_id, token_hash, label,
                   capabilities, created_at, expires_at, revoked_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(&new.token_hash)
    .bind(new.label.as_deref())
    .bind(&capabilities)
    .bind(expires_at)
    .bind(parent_token_id.map(|id| id.0))
    .bind(grant_id.0)
    .bind(delegate_id.0)
    .fetch_optional(pool)
    .await
    .map_err(map_token_err)?
    .ok_or(StoreError::NotFound)?;
    row_to_token(&row)
}

pub async fn get_by_id(pool: &PgPool, id: ApiTokenId) -> Result<ApiToken, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, member_id, app_installation_id, token_hash, label, capabilities,
                created_at, expires_at, revoked_at
         FROM maidan_api_tokens
         WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_token(&row)
}

pub async fn get_active_by_hash(pool: &PgPool, token_hash: &str) -> Result<ApiToken, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, member_id, app_installation_id, token_hash, label,
                capabilities, created_at, expires_at, revoked_at
         FROM maidan_api_tokens
         WHERE token_hash = $1
           AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > NOW())
           AND (
             delegation_grant_id IS NULL
             OR EXISTS (
               SELECT 1 FROM maidan_delegation_grants g
               WHERE g.id = maidan_api_tokens.delegation_grant_id
                 AND g.revoked_at IS NULL AND g.expires_at > NOW()
             )
           )
           AND (
             app_installation_id IS NULL
             OR EXISTS (
               SELECT 1 FROM maidan_app_installations i
               WHERE i.id = maidan_api_tokens.app_installation_id AND i.revoked_at IS NULL
             )
           )",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_token(&row)
}

pub async fn workspace_has_active_capability(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    capability: &str,
) -> Result<bool, StoreError> {
    let needle = format!("%\"{capability}\"%");
    let found = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM maidan_api_tokens
         WHERE workspace_id = $1
           AND revoked_at IS NULL
           AND capabilities LIKE $2
         LIMIT 1",
    )
    .bind(workspace_id.0)
    .bind(needle)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

pub async fn list_for_member(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<ApiToken>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, member_id, app_installation_id, token_hash, label, capabilities,
                created_at, expires_at, revoked_at
         FROM maidan_api_tokens
         WHERE workspace_id = $1 AND member_id = $2
         ORDER BY created_at DESC",
    )
    .bind(workspace_id.0)
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_token).collect()
}

/// Revoke a token **and every token derived from it**.
///
/// A derived token inherits every limit the parent carried — app installation, per-token quotas — because re-issuing was
/// otherwise a way to shed a bound. Revocation is the ultimate limit, and it
/// was the one dimension still leaking: the parent link existed only in audit
/// metadata, so a child outlived the credential it was minted from. You revoke
/// a parent because it leaked, and whoever held it could have minted children
/// from it; those are equally compromised.
///
/// **Cascade at revoke time, not at auth time.** Writing `revoked_at` across the
/// subtree is one traversal; checking the chain on every request would put a
/// recursive query in the hot auth path. Attenuation requires a live parent, so
/// a child cannot appear after its parent is revoked.
///
/// Already-revoked rows are left alone, so their original revocation time
/// survives. `NotFound` if the root was already revoked or does not exist —
/// unchanged from before.
pub async fn revoke(pool: &PgPool, id: ApiTokenId) -> Result<ApiToken, StoreError> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;
    // The root must still be live, and the caller gets its row back.
    let row = sqlx::query(
        "UPDATE maidan_api_tokens
         SET revoked_at = $2
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING id, workspace_id, member_id, app_installation_id, token_hash, label, capabilities,
                   created_at, expires_at, revoked_at",
    )
    .bind(id.0)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    sqlx::query(
        "WITH RECURSIVE subtree AS (
             SELECT id FROM maidan_api_tokens WHERE parent_token_id = $1
             UNION
             SELECT t.id FROM maidan_api_tokens t
             JOIN subtree s ON t.parent_token_id = s.id
         )
         UPDATE maidan_api_tokens SET revoked_at = $2
         WHERE id IN (SELECT id FROM subtree) AND revoked_at IS NULL",
    )
    .bind(id.0)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    row_to_token(&row)
}

fn map_token_err(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db) = err {
        if db.is_unique_violation() {
            return StoreError::Conflict("token hash already exists".into());
        }
    }
    StoreError::Database(err)
}

fn row_to_token(row: &sqlx::postgres::PgRow) -> Result<ApiToken, StoreError> {
    let capabilities_json: String = row.get("capabilities");
    let capabilities: Vec<String> = serde_json::from_str(&capabilities_json).map_err(|e| {
        StoreError::InvalidInput(format!("invalid capabilities JSON in database: {e}"))
    })?;
    Ok(ApiToken {
        id: ApiTokenId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        app_installation_id: row
            .get::<Option<Uuid>, _>("app_installation_id")
            .map(AppInstallationId),
        token_hash: row.get("token_hash"),
        label: row.get("label"),
        capabilities,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        expires_at: row.get::<Option<DateTime<Utc>>, _>("expires_at"),
        revoked_at: row.get::<Option<DateTime<Utc>>, _>("revoked_at"),
    })
}
