use chrono::{DateTime, Utc};
use maidan_types::{NewShareTicket, ShareTicket, ShareTicketId, WorkspaceId};
use sqlx::{Row, SqlitePool};

use crate::{share_tickets, StoreError};

const COLUMNS: &str = "id, workspace_id, channel_id, owner_id, created_by, token_hash, expires_at, revoked_at, created_at";

pub async fn create(pool: &SqlitePool, new: NewShareTicket) -> Result<ShareTicket, StoreError> {
    let mut conn = pool.acquire().await?;
    create_on(&mut conn, new).await
}

pub(crate) async fn create_on(
    conn: &mut sqlx::SqliteConnection,
    new: NewShareTicket,
) -> Result<ShareTicket, StoreError> {
    let artifacts = share_tickets::validate_new(&new, Utc::now())?;
    let id = ShareTicketId::new();
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let scope_valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1
            FROM maidan_channels c
            JOIN maidan_members owner ON owner.id = ?3
            JOIN maidan_members creator ON creator.id = ?4
            WHERE c.id = ?1 AND c.workspace_id = ?2
              AND owner.workspace_id = ?2 AND owner.tombstoned_at IS NULL
              AND creator.workspace_id = ?2 AND creator.tombstoned_at IS NULL
              AND c.tombstoned_at IS NULL
        )",
    )
    .bind(new.channel_id.0)
    .bind(new.workspace_id.0)
    .bind(new.owner_id.0)
    .bind(new.created_by.0)
    .fetch_one(&mut *tx)
    .await?;
    if !scope_valid {
        return Err(StoreError::InvalidInput(
            "share ticket channel, owner, and creator must be live in the same workspace".into(),
        ));
    }
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_share_tickets
            (id, workspace_id, channel_id, owner_id, created_by, token_hash, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         RETURNING {COLUMNS}"
    ))
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(new.owner_id.0)
    .bind(new.created_by.0)
    .bind(&new.token_hash)
    .bind(new.expires_at)
    .fetch_one(&mut *tx)
    .await?;
    let ticket = row_to_ticket(&row);
    for sha in artifacts {
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO maidan_share_ticket_artifacts (ticket_id, sha256)
             SELECT ?1, r.sha256
             FROM maidan_artifact_refs r
             JOIN maidan_artifacts a ON a.sha256 = r.sha256
             WHERE r.workspace_id = ?2 AND r.sha256 = ?3",
        )
        .bind(id.0)
        .bind(new.workspace_id.0)
        .bind(&sha)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(StoreError::InvalidInput(format!(
                "artifact {sha} is not linked to the ticket workspace"
            )));
        }
    }
    tx.commit().await?;
    Ok(ticket)
}

pub async fn get(pool: &SqlitePool, id: ShareTicketId) -> Result<ShareTicket, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM maidan_share_tickets WHERE id = ?1"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_ticket(&row))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<ShareTicket>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM maidan_share_tickets
         WHERE workspace_id = ?1 ORDER BY created_at DESC, id DESC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_ticket).collect())
}

pub async fn resolve(
    pool: &SqlitePool,
    token_hash: &str,
    now: DateTime<Utc>,
) -> Result<ShareTicket, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM maidan_share_tickets
         WHERE token_hash = ?1 AND revoked_at IS NULL
           AND datetime(expires_at) > datetime(?2)"
    ))
    .bind(token_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_ticket(&row))
}

pub async fn revoke(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    id: ShareTicketId,
) -> Result<bool, StoreError> {
    let mut conn = pool.acquire().await?;
    revoke_on(&mut conn, workspace_id, id).await
}

pub(crate) async fn revoke_on(
    conn: &mut sqlx::SqliteConnection,
    workspace_id: WorkspaceId,
    id: ShareTicketId,
) -> Result<bool, StoreError> {
    let result = sqlx::query(
        "UPDATE maidan_share_tickets SET revoked_at = datetime('now')
         WHERE id = ?1 AND workspace_id = ?2 AND revoked_at IS NULL",
    )
    .bind(id.0)
    .bind(workspace_id.0)
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn list_artifacts(
    pool: &SqlitePool,
    id: ShareTicketId,
) -> Result<Vec<String>, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT sha256 FROM maidan_share_ticket_artifacts
         WHERE ticket_id = ?1 ORDER BY sha256",
    )
    .bind(id.0)
    .fetch_all(pool)
    .await?)
}

pub async fn allows_artifact(
    pool: &SqlitePool,
    id: ShareTicketId,
    sha256: &str,
    now: DateTime<Utc>,
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM maidan_share_ticket_artifacts a
            JOIN maidan_share_tickets t ON t.id = a.ticket_id
            WHERE a.ticket_id = ?1 AND a.sha256 = ?2
              AND t.revoked_at IS NULL
              AND datetime(t.expires_at) > datetime(?3)
        )",
    )
    .bind(id.0)
    .bind(sha256)
    .bind(now)
    .fetch_one(pool)
    .await?)
}

fn row_to_ticket(row: &sqlx::sqlite::SqliteRow) -> ShareTicket {
    ShareTicket {
        id: ShareTicketId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        channel_id: maidan_types::ChannelId(row.get("channel_id")),
        owner_id: maidan_types::MemberId(row.get("owner_id")),
        created_by: maidan_types::MemberId(row.get("created_by")),
        token_hash: row.get("token_hash"),
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
        created_at: row.get("created_at"),
    }
}

/// [`create`], with its audit row in the same transaction (D-A).
pub async fn create_audited(
    pool: &SqlitePool,
    new: NewShareTicket,
    audit: crate::AuditFor<ShareTicket>,
) -> Result<ShareTicket, StoreError> {
    let mut tx = pool.begin().await?;
    let ticket = create_on(&mut tx, new).await?;
    super::audit::append_counted(&mut tx, audit(&ticket)).await?;
    tx.commit().await?;
    Ok(ticket)
}

/// [`revoke`], with its audit row in the same transaction. Nothing is recorded
/// when there was no live ticket to revoke.
pub async fn revoke_audited(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    id: ShareTicketId,
    audit: maidan_types::NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let revoked = revoke_on(&mut tx, workspace_id, id).await?;
    if revoked {
        super::audit::append_counted(&mut tx, audit).await?;
    }
    tx.commit().await?;
    Ok(revoked)
}
