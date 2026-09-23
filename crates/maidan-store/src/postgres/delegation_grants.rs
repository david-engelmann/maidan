use maidan_types::{DelegationGrant, DelegationGrantId, MemberId, NewDelegationGrant, WorkspaceId};
use sqlx::{PgPool, Row};

use crate::{delegation_grants, StoreError};

const COLUMNS: &str = "id, workspace_id, subject_id, delegate_id, capabilities, purpose, authorized_by, expires_at, revoked_at, created_at";

pub async fn create(pool: &PgPool, new: NewDelegationGrant) -> Result<DelegationGrant, StoreError> {
    let (capabilities, purpose) = delegation_grants::validate_new(&new, chrono::Utc::now())?;
    let encoded = serde_json::to_string(&capabilities)?;
    let scope_valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maidan_members subject
         JOIN maidan_members delegate ON delegate.id = $3
         JOIN maidan_members authorizer ON authorizer.id = $4
         WHERE subject.id = $2
           AND subject.workspace_id = $1 AND subject.tombstoned_at IS NULL
           AND delegate.workspace_id = $1 AND delegate.tombstoned_at IS NULL
           AND authorizer.workspace_id = $1 AND authorizer.tombstoned_at IS NULL)",
    )
    .bind(new.workspace_id.0)
    .bind(new.subject_id.0)
    .bind(new.delegate_id.0)
    .bind(new.authorized_by.0)
    .fetch_one(pool)
    .await?;
    if !scope_valid {
        return Err(StoreError::InvalidInput(
            "delegation subject, delegate, and authorizer must be live in the same workspace"
                .into(),
        ));
    }
    let id = DelegationGrantId::new();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_delegation_grants
         (id, workspace_id, subject_id, delegate_id, capabilities, purpose, authorized_by, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING {COLUMNS}"
    ))
    .bind(id.0).bind(new.workspace_id.0).bind(new.subject_id.0).bind(new.delegate_id.0)
    .bind(encoded).bind(purpose).bind(new.authorized_by.0).bind(new.expires_at)
    .fetch_one(pool).await?;
    row_to_grant(&row)
}

pub async fn get(pool: &PgPool, id: DelegationGrantId) -> Result<DelegationGrant, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM maidan_delegation_grants WHERE id=$1"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_grant(&row)
}

pub async fn list(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<DelegationGrant>, StoreError> {
    let rows = sqlx::query(&format!("SELECT {COLUMNS} FROM maidan_delegation_grants WHERE workspace_id=$1 ORDER BY created_at DESC, id DESC"))
        .bind(workspace_id.0).fetch_all(pool).await?;
    rows.iter().map(row_to_grant).collect()
}

pub async fn revoke(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    id: DelegationGrantId,
) -> Result<bool, StoreError> {
    let now = chrono::Utc::now();
    let mut tx = pool.begin().await?;
    let result = sqlx::query("UPDATE maidan_delegation_grants SET revoked_at=$3 WHERE id=$1 AND workspace_id=$2 AND revoked_at IS NULL")
        .bind(id.0).bind(workspace_id.0).bind(now).execute(&mut *tx).await?;
    if result.rows_affected() != 1 {
        return Ok(false);
    }
    sqlx::query(
        "WITH RECURSIVE subtree AS (
             SELECT id FROM maidan_api_tokens WHERE delegation_grant_id = $1
             UNION
             SELECT t.id FROM maidan_api_tokens t JOIN subtree s ON t.parent_token_id = s.id
         )
         UPDATE maidan_api_tokens SET revoked_at = $2
         WHERE id IN (SELECT id FROM subtree) AND revoked_at IS NULL",
    )
    .bind(id.0)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

fn row_to_grant(row: &sqlx::postgres::PgRow) -> Result<DelegationGrant, StoreError> {
    Ok(DelegationGrant {
        id: DelegationGrantId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        subject_id: MemberId(row.get("subject_id")),
        delegate_id: MemberId(row.get("delegate_id")),
        capabilities: serde_json::from_str(row.get("capabilities"))?,
        purpose: row.get("purpose"),
        authorized_by: MemberId(row.get("authorized_by")),
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
        created_at: row.get("created_at"),
    })
}
