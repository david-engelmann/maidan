//! SCIM 2.0 Groups: the `maidan_scim_groups` table and its membership rows.
//!
//! Every write commits in one transaction with its audit row (D-A). A member
//! is added only when it is a SCIM user of the group's workspace; the
//! membership row's foreign keys on `(id, workspace_id)` refuse anything else
//! even if that check is skipped.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use maidan_types::{
    MemberId, NewAuditEvent, NewScimGroup, ScimGroup, ScimGroupChange, ScimGroupId,
    ScimGroupMember, ScimGroupWrite, ScimMembersOp, WorkspaceId,
};
use sqlx::{Row, SqliteConnection, SqlitePool};
use uuid::Uuid;

use super::audit;
use crate::{error::StoreError, AuditFor};

const COLS: &str = "id, workspace_id, display_name, external_id, created_at, updated_at";

fn row_to_group(row: &sqlx::sqlite::SqliteRow, members: Vec<ScimGroupMember>) -> ScimGroup {
    ScimGroup {
        id: ScimGroupId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        display_name: row.get("display_name"),
        external_id: row.get("external_id"),
        members,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

async fn members_on(
    conn: &mut SqliteConnection,
    group_id: ScimGroupId,
) -> Result<Vec<ScimGroupMember>, StoreError> {
    let rows = sqlx::query(
        "SELECT gm.member_id, m.handle
         FROM maidan_scim_group_members gm
         JOIN maidan_members m ON m.id = gm.member_id
         WHERE gm.group_id = ?
         ORDER BY m.handle ASC, gm.member_id ASC",
    )
    .bind(group_id.0)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .iter()
        .map(|row| ScimGroupMember {
            member_id: MemberId(row.get::<Uuid, _>("member_id")),
            handle: row.get("handle"),
        })
        .collect())
}

async fn get_on(
    conn: &mut SqliteConnection,
    workspace_id: WorkspaceId,
    id: ScimGroupId,
) -> Result<Option<ScimGroup>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_scim_groups WHERE id = ? AND workspace_id = ?"
    ))
    .bind(id.0)
    .bind(workspace_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let members = members_on(conn, id).await?;
    Ok(Some(row_to_group(&row, members)))
}

pub async fn get(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    id: ScimGroupId,
) -> Result<Option<ScimGroup>, StoreError> {
    let mut conn = pool.acquire().await?;
    get_on(&mut conn, workspace_id, id).await
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<ScimGroup>, StoreError> {
    let groups = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_scim_groups WHERE workspace_id = ?
         ORDER BY created_at ASC, id ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    // One query for every membership in the workspace, not one per group.
    let memberships = sqlx::query(
        "SELECT gm.group_id, gm.member_id, m.handle
         FROM maidan_scim_group_members gm
         JOIN maidan_members m ON m.id = gm.member_id
         WHERE gm.workspace_id = ?
         ORDER BY m.handle ASC, gm.member_id ASC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    let mut by_group: HashMap<Uuid, Vec<ScimGroupMember>> = HashMap::new();
    for row in &memberships {
        by_group
            .entry(row.get::<Uuid, _>("group_id"))
            .or_default()
            .push(ScimGroupMember {
                member_id: MemberId(row.get::<Uuid, _>("member_id")),
                handle: row.get("handle"),
            });
    }
    Ok(groups
        .iter()
        .map(|row| {
            let members = by_group
                .remove(&row.get::<Uuid, _>("id"))
                .unwrap_or_default();
            row_to_group(row, members)
        })
        .collect())
}

/// `true` when the member was added, `false` when it already belonged.
async fn add_on(
    conn: &mut SqliteConnection,
    workspace_id: WorkspaceId,
    group_id: ScimGroupId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let provisioned: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM maidan_scim_users WHERE member_id = ? AND workspace_id = ?",
    )
    .bind(member_id.0)
    .bind(workspace_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    if provisioned.is_none() {
        // The same answer whether the id is unknown or another workspace's.
        return Err(StoreError::InvalidInput(format!(
            "{member_id} is not a user of this workspace"
        )));
    }
    let done = sqlx::query(
        "INSERT INTO maidan_scim_group_members (group_id, member_id, workspace_id, added_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (group_id, member_id) DO NOTHING",
    )
    .bind(group_id.0)
    .bind(member_id.0)
    .bind(workspace_id.0)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *conn)
    .await?;
    Ok(done.rows_affected() > 0)
}

async fn remove_on(
    conn: &mut SqliteConnection,
    group_id: ScimGroupId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM maidan_scim_group_members WHERE group_id = ? AND member_id = ?")
        .bind(group_id.0)
        .bind(member_id.0)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn member_ids_on(
    conn: &mut SqliteConnection,
    group_id: ScimGroupId,
) -> Result<BTreeSet<Uuid>, StoreError> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT member_id FROM maidan_scim_group_members WHERE group_id = ?")
            .bind(group_id.0)
            .fetch_all(&mut *conn)
            .await?;
    Ok(ids.into_iter().collect())
}

async fn apply_members_on(
    conn: &mut SqliteConnection,
    workspace_id: WorkspaceId,
    group_id: ScimGroupId,
    ops: &[ScimMembersOp],
) -> Result<(), StoreError> {
    for op in ops {
        match op {
            ScimMembersOp::Add(ids) => {
                for id in ids {
                    add_on(conn, workspace_id, group_id, *id).await?;
                }
            }
            ScimMembersOp::Remove(ids) => {
                for id in ids {
                    remove_on(conn, group_id, *id).await?;
                }
            }
            ScimMembersOp::Replace(ids) => {
                let wanted: BTreeSet<Uuid> = ids.iter().map(|id| id.0).collect();
                for current in member_ids_on(conn, group_id).await? {
                    if !wanted.contains(&current) {
                        remove_on(conn, group_id, MemberId(current)).await?;
                    }
                }
                for id in wanted {
                    add_on(conn, workspace_id, group_id, MemberId(id)).await?;
                }
            }
        }
    }
    Ok(())
}

pub async fn create_audited(
    pool: &SqlitePool,
    new: NewScimGroup,
    audit_for: AuditFor<ScimGroup>,
) -> Result<ScimGroup, StoreError> {
    let id = ScimGroupId::new();
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO maidan_scim_groups
             (id, workspace_id, display_name, external_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(&new.display_name)
    .bind(new.external_id.as_deref())
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    apply_members_on(
        &mut tx,
        new.workspace_id,
        id,
        &[ScimMembersOp::Add(new.members)],
    )
    .await?;
    let group = get_on(&mut tx, new.workspace_id, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    audit::append_counted(&mut tx, audit_for(&group)).await?;
    tx.commit().await?;
    Ok(group)
}

pub async fn update_audited(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    id: ScimGroupId,
    change: ScimGroupChange,
    audit_for: AuditFor<ScimGroupWrite>,
) -> Result<Option<ScimGroupWrite>, StoreError> {
    let mut tx = pool.begin().await?;
    let Some(current) = get_on(&mut tx, workspace_id, id).await? else {
        return Ok(None);
    };
    let display_name = change.display_name.unwrap_or(current.display_name);
    let external_id = change.external_id.unwrap_or(current.external_id);
    sqlx::query(
        "UPDATE maidan_scim_groups SET display_name = ?, external_id = ?, updated_at = ?
         WHERE id = ? AND workspace_id = ?",
    )
    .bind(&display_name)
    .bind(external_id.as_deref())
    .bind(Utc::now().to_rfc3339())
    .bind(id.0)
    .bind(workspace_id.0)
    .execute(&mut *tx)
    .await?;
    apply_members_on(&mut tx, workspace_id, id, &change.members).await?;
    let group = get_on(&mut tx, workspace_id, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let write = net_change(&current.members, group);
    audit::append_counted(&mut tx, audit_for(&write)).await?;
    tx.commit().await?;
    Ok(Some(write))
}

/// The membership difference between before and after, whatever the
/// operations were: a PatchOp that adds and then removes the same member
/// records nothing for it.
fn net_change(before: &[ScimGroupMember], group: ScimGroup) -> ScimGroupWrite {
    let before: BTreeSet<Uuid> = before.iter().map(|m| m.member_id.0).collect();
    let after: BTreeSet<Uuid> = group.members.iter().map(|m| m.member_id.0).collect();
    ScimGroupWrite {
        added: after.difference(&before).map(|id| MemberId(*id)).collect(),
        removed: before.difference(&after).map(|id| MemberId(*id)).collect(),
        group,
    }
}

pub async fn delete_audited(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    id: ScimGroupId,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let done = sqlx::query("DELETE FROM maidan_scim_groups WHERE id = ? AND workspace_id = ?")
        .bind(id.0)
        .bind(workspace_id.0)
        .execute(&mut *tx)
        .await?;
    if done.rows_affected() == 0 {
        return Ok(false);
    }
    audit::append_counted(&mut tx, event).await?;
    tx.commit().await?;
    Ok(true)
}
