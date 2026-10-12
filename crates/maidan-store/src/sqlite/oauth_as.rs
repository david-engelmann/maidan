//! SQLite backing for the OAuth 2.1 authorization server (`docs/OAuth.md`).
//!
//! Mirrors [`crate::postgres::oauth_as`]. TTL comparisons bind `Utc::now()`
//! rather than a SQL `strftime` so both sides use sqlx's own `DateTime<Utc>`
//! text encoding — keeping the lexical comparison consistent regardless of
//! format.

use chrono::{DateTime, Utc};
use maidan_types::{
    ApiToken, MemberId, NewApiToken, NewOAuthAuthorizationCode, NewOAuthClient, NewOAuthGrant,
    NewOAuthPendingRequest, OAuthAuthorizationCode, OAuthClient, OAuthGrant, OAuthGrantId,
    OAuthPendingRequest, OAuthPendingRequestId, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const CLIENT_COLUMNS: &str =
    "id, client_id, name, redirect_uris, client_secret_hash, allowed_scopes, revoked_at";
const CODE_COLUMNS: &str = "code_hash, client_id, member_id, workspace_id, redirect_uri, code_challenge, scope, resource, expires_at, used_at";
const GRANT_COLUMNS: &str = "id, client_id, member_id, workspace_id, scope, lineage_id, revoked_at";

fn scopes_from(json: &str) -> Result<Vec<String>, StoreError> {
    serde_json::from_str(json)
        .map_err(|e| StoreError::InvalidInput(format!("invalid scope JSON in database: {e}")))
}

fn row_to_client(row: &sqlx::sqlite::SqliteRow) -> Result<OAuthClient, StoreError> {
    Ok(OAuthClient {
        id: row.get::<Uuid, _>("id"),
        client_id: row.get("client_id"),
        name: row.get("name"),
        redirect_uris: scopes_from(&row.get::<String, _>("redirect_uris"))?,
        client_secret_hash: row.get("client_secret_hash"),
        allowed_scopes: scopes_from(&row.get::<String, _>("allowed_scopes"))?,
        revoked_at: row.get::<Option<DateTime<Utc>>, _>("revoked_at"),
    })
}

fn row_to_code(row: &sqlx::sqlite::SqliteRow) -> Result<OAuthAuthorizationCode, StoreError> {
    Ok(OAuthAuthorizationCode {
        code_hash: row.get("code_hash"),
        client_id: row.get("client_id"),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        redirect_uri: row.get("redirect_uri"),
        code_challenge: row.get("code_challenge"),
        scope: scopes_from(&row.get::<String, _>("scope"))?,
        resource: row.get("resource"),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at"),
        used_at: row.get::<Option<DateTime<Utc>>, _>("used_at"),
    })
}

fn row_to_grant(row: &sqlx::sqlite::SqliteRow) -> Result<OAuthGrant, StoreError> {
    Ok(OAuthGrant {
        id: OAuthGrantId(row.get::<Uuid, _>("id")),
        client_id: row.get("client_id"),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        scope: scopes_from(&row.get::<String, _>("scope"))?,
        lineage_id: row.get::<Uuid, _>("lineage_id"),
        revoked_at: row.get::<Option<DateTime<Utc>>, _>("revoked_at"),
    })
}

pub async fn insert_client(
    pool: &SqlitePool,
    new: NewOAuthClient,
) -> Result<OAuthClient, StoreError> {
    let id = Uuid::now_v7();
    let redirect_uris = serde_json::to_string(&new.redirect_uris)?;
    let allowed_scopes = serde_json::to_string(&new.allowed_scopes)?;
    let row = sqlx::query(&format!(
        "INSERT INTO oauth_clients
            (id, client_id, name, redirect_uris, client_secret_hash, allowed_scopes,
             created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING {CLIENT_COLUMNS}"
    ))
    .bind(id)
    .bind(&new.client_id)
    .bind(&new.name)
    .bind(&redirect_uris)
    .bind(new.client_secret_hash.as_deref())
    .bind(&allowed_scopes)
    .bind(Utc::now())
    .fetch_one(pool)
    .await?;
    row_to_client(&row)
}

pub async fn get_client_by_client_id(
    pool: &SqlitePool,
    client_id: &str,
) -> Result<Option<OAuthClient>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {CLIENT_COLUMNS} FROM oauth_clients
         WHERE client_id = ? AND revoked_at IS NULL"
    ))
    .bind(client_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| row_to_client(&r)).transpose()
}

pub async fn insert_code(
    pool: &SqlitePool,
    new: NewOAuthAuthorizationCode,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM oauth_authorization_codes WHERE expires_at <= ?")
        .bind(Utc::now())
        .execute(pool)
        .await?;
    let scope = serde_json::to_string(&new.scope)?;
    sqlx::query(
        "INSERT INTO oauth_authorization_codes
            (code_hash, client_id, member_id, workspace_id, redirect_uri,
             code_challenge, code_challenge_method, scope, resource, created_at,
             expires_at)
         VALUES (?, ?, ?, ?, ?, ?, 'S256', ?, ?, ?, ?)",
    )
    .bind(&new.code_hash)
    .bind(&new.client_id)
    .bind(new.member_id.0)
    .bind(new.workspace_id.0)
    .bind(&new.redirect_uri)
    .bind(&new.code_challenge)
    .bind(&scope)
    .bind(new.resource.as_deref())
    .bind(Utc::now())
    .bind(new.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_code(
    pool: &SqlitePool,
    code_hash: &str,
) -> Result<Option<OAuthAuthorizationCode>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {CODE_COLUMNS} FROM oauth_authorization_codes WHERE code_hash = ?"
    ))
    .bind(code_hash)
    .fetch_optional(pool)
    .await?;
    row.map(|r| row_to_code(&r)).transpose()
}

pub async fn consume_code(
    pool: &SqlitePool,
    code_hash: &str,
) -> Result<Option<OAuthAuthorizationCode>, StoreError> {
    // One statement: only a live code flips to used, so two concurrent
    // exchanges cannot both win. Call only after the code validated —
    // a wrong verifier must not burn the real client's code.
    let now = Utc::now();
    let row = sqlx::query(&format!(
        "UPDATE oauth_authorization_codes
         SET used_at = ?
         WHERE code_hash = ? AND used_at IS NULL AND expires_at > ?
         RETURNING {CODE_COLUMNS}"
    ))
    .bind(now)
    .bind(code_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    row.map(|r| row_to_code(&r)).transpose()
}

pub async fn insert_grant(pool: &SqlitePool, new: NewOAuthGrant) -> Result<OAuthGrant, StoreError> {
    let mut conn = pool.acquire().await?;
    insert_grant_on(&mut conn, new).await
}

pub(crate) async fn insert_grant_on(
    conn: &mut sqlx::SqliteConnection,
    new: NewOAuthGrant,
) -> Result<OAuthGrant, StoreError> {
    let id = Uuid::now_v7();
    let scope = serde_json::to_string(&new.scope)?;
    let row = sqlx::query(&format!(
        "INSERT INTO oauth_grants
            (id, client_id, member_id, workspace_id, scope, lineage_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING {GRANT_COLUMNS}"
    ))
    .bind(id)
    .bind(&new.client_id)
    .bind(new.member_id.0)
    .bind(new.workspace_id.0)
    .bind(&scope)
    .bind(new.lineage_id)
    .bind(Utc::now())
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db) = e {
            if db.is_unique_violation() {
                return StoreError::Conflict("oauth grant already exists".into());
            }
        }
        StoreError::Database(e)
    })?;
    row_to_grant(&row)
}

pub async fn insert_grant_audited(
    pool: &SqlitePool,
    new: NewOAuthGrant,
    audit: crate::AuditFor<OAuthGrant>,
) -> Result<OAuthGrant, StoreError> {
    let mut tx = pool.begin().await?;
    let grant = insert_grant_on(&mut tx, new).await?;
    super::audit::append_on(&mut tx, audit(&grant))
        .await
        .inspect_err(|_| crate::attribution::count_audit_write_failure())?;
    tx.commit().await?;
    Ok(grant)
}

pub async fn get_grant(
    pool: &SqlitePool,
    grant_id: OAuthGrantId,
) -> Result<Option<OAuthGrant>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {GRANT_COLUMNS} FROM oauth_grants
         WHERE id = ? AND revoked_at IS NULL"
    ))
    .bind(grant_id.0)
    .fetch_optional(pool)
    .await?;
    row.map(|r| row_to_grant(&r)).transpose()
}

pub async fn find_grant(
    pool: &SqlitePool,
    client_id: &str,
    member_id: MemberId,
    workspace_id: WorkspaceId,
    scope: &[String],
) -> Result<Option<OAuthGrant>, StoreError> {
    let scope_json = serde_json::to_string(scope)?;
    let row = sqlx::query(&format!(
        "SELECT {GRANT_COLUMNS} FROM oauth_grants
         WHERE client_id = ? AND member_id = ? AND workspace_id = ?
           AND scope = ? AND revoked_at IS NULL
         ORDER BY created_at DESC
         LIMIT 1"
    ))
    .bind(client_id)
    .bind(member_id.0)
    .bind(workspace_id.0)
    .bind(&scope_json)
    .fetch_optional(pool)
    .await?;
    row.map(|r| row_to_grant(&r)).transpose()
}

pub async fn revoke_grant(pool: &SqlitePool, grant_id: OAuthGrantId) -> Result<(), StoreError> {
    sqlx::query("UPDATE oauth_grants SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(Utc::now())
        .bind(grant_id.0)
        .execute(pool)
        .await?;
    Ok(())
}

/// The capability an OAuth token must never carry. This mirrors
/// `maidan_auth::capability::APPROVAL_GRANT`; the store cannot depend on
/// maidan-auth (it depends on the store), so the string is repeated here
/// rather than imported.
const APPROVAL_GRANT: &str = "approval:grant";

/// Capabilities with [`APPROVAL_GRANT`] removed. An OAuth token never accepts
/// an approval gate, whatever the caller asked for.
fn sanitized_capabilities(capabilities: &[String]) -> Vec<String> {
    capabilities
        .iter()
        .filter(|c| c.as_str() != APPROVAL_GRANT)
        .cloned()
        .collect()
}

pub async fn mint_token(
    pool: &SqlitePool,
    new: NewApiToken,
    grant_id: OAuthGrantId,
) -> Result<ApiToken, StoreError> {
    let mut conn = pool.acquire().await?;
    mint_token_on(&mut conn, new, grant_id).await
}

pub(crate) async fn mint_token_on(
    conn: &mut sqlx::SqliteConnection,
    new: NewApiToken,
    grant_id: OAuthGrantId,
) -> Result<ApiToken, StoreError> {
    let id = Uuid::now_v7();
    let now = Utc::now();
    let capabilities = serde_json::to_string(&sanitized_capabilities(&new.capabilities))?;
    let row = sqlx::query(
        "INSERT INTO maidan_api_tokens
            (id, workspace_id, member_id, app_installation_id, token_hash, label,
             capabilities, created_at, expires_at, oauth_grant_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id, workspace_id, member_id, app_installation_id, token_hash, label,
                   capabilities, created_at, expires_at, revoked_at, delegation_grant_id,
                   oauth_grant_id",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(new.app_installation_id.map(|i| i.0))
    .bind(&new.token_hash)
    .bind(new.label.as_deref())
    .bind(&capabilities)
    .bind(now)
    .bind(new.expires_at)
    .bind(grant_id.0)
    .fetch_one(&mut *conn)
    .await
    .map_err(crate::sqlite::tokens::map_token_err)?;
    crate::sqlite::tokens::row_to_token(&row)
}

pub async fn mint_token_audited(
    pool: &SqlitePool,
    new: NewApiToken,
    grant_id: OAuthGrantId,
    audit: crate::AuditFor<ApiToken>,
) -> Result<ApiToken, StoreError> {
    let mut tx = pool.begin().await?;
    let token = mint_token_on(&mut tx, new, grant_id).await?;
    super::audit::append_on(&mut tx, audit(&token))
        .await
        .inspect_err(|_| crate::attribution::count_audit_write_failure())?;
    tx.commit().await?;
    Ok(token)
}

const PENDING_COLUMNS: &str = "id, client_id, member_id, workspace_id, redirect_uri, code_challenge, scope, resource, state, expires_at";

fn row_to_pending(row: &sqlx::sqlite::SqliteRow) -> Result<OAuthPendingRequest, StoreError> {
    Ok(OAuthPendingRequest {
        id: OAuthPendingRequestId(
            Uuid::parse_str(&row.get::<String, _>("id"))
                .map_err(|e| StoreError::InvalidInput(format!("bad pending id: {e}")))?,
        ),
        client_id: row.get("client_id"),
        member_id: MemberId(
            Uuid::parse_str(&row.get::<String, _>("member_id"))
                .map_err(|e| StoreError::InvalidInput(format!("bad member id: {e}")))?,
        ),
        workspace_id: WorkspaceId(
            Uuid::parse_str(&row.get::<String, _>("workspace_id"))
                .map_err(|e| StoreError::InvalidInput(format!("bad workspace id: {e}")))?,
        ),
        redirect_uri: row.get("redirect_uri"),
        code_challenge: row.get("code_challenge"),
        scope: scopes_from(&row.get::<String, _>("scope"))?,
        resource: row.get("resource"),
        state: row.get("state"),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at"),
    })
}

/// Store a validated authorize request while the member decides.
pub async fn create_pending_request(
    pool: &SqlitePool,
    new: NewOAuthPendingRequest,
) -> Result<OAuthPendingRequest, StoreError> {
    let id = Uuid::now_v7();
    let scope = serde_json::to_string(&new.scope)?;
    let now = Utc::now().to_rfc3339();
    let expires = new.expires_at.to_rfc3339();
    sqlx::query(
        "INSERT INTO oauth_pending_requests
            (id, client_id, member_id, workspace_id, redirect_uri, code_challenge,
             scope, resource, \"state\", created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&new.client_id)
    .bind(new.member_id.0.to_string())
    .bind(new.workspace_id.0.to_string())
    .bind(&new.redirect_uri)
    .bind(&new.code_challenge)
    .bind(&scope)
    .bind(new.resource.as_deref())
    .bind(&new.state)
    .bind(&now)
    .bind(&expires)
    .execute(pool)
    .await?;
    get_pending_request(pool, OAuthPendingRequestId(id))
        .await?
        .ok_or(StoreError::NotFound)
}

/// A pending request by id, or `None`.
pub async fn get_pending_request(
    pool: &SqlitePool,
    id: OAuthPendingRequestId,
) -> Result<Option<OAuthPendingRequest>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {PENDING_COLUMNS} FROM oauth_pending_requests WHERE id = ?"
    ))
    .bind(id.0.to_string())
    .fetch_optional(pool)
    .await?;
    row.map(|r| row_to_pending(&r)).transpose()
}

/// Delete a pending request (single use).
pub async fn delete_pending_request(
    pool: &SqlitePool,
    id: OAuthPendingRequestId,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM oauth_pending_requests WHERE id = ?")
        .bind(id.0.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically consume a pending request: delete it only if it exists, is
/// unexpired, and belongs to the given member and workspace. Returns the
/// row if consumed, `None` otherwise. Two concurrent consumes for one
/// request yield exactly one row.
pub async fn consume_pending_request(
    pool: &SqlitePool,
    id: OAuthPendingRequestId,
    member_id: maidan_types::MemberId,
    workspace_id: maidan_types::WorkspaceId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<OAuthPendingRequest>, StoreError> {
    let row = sqlx::query(&format!(
        "DELETE FROM oauth_pending_requests
         WHERE id = ? AND member_id = ? AND workspace_id = ? AND expires_at > ?
         RETURNING {PENDING_COLUMNS}"
    ))
    .bind(id.0.to_string())
    .bind(member_id.0.to_string())
    .bind(workspace_id.0.to_string())
    .bind(now)
    .fetch_optional(pool)
    .await?;
    row.map(|r| row_to_pending(&r)).transpose()
}
