//! Planner fan-out (#1421): a plan parent's children, made in one transaction
//! by the planner holding the parent's claim. SQLite twin of the Postgres
//! module.

use chrono::{DateTime, Utc};
use maidan_types::{
    ClaimLeaseId, Event, MemberId, PlanOutcome, PlanState, PlannedChild, SpawnAxis, SpawnDenial,
    SubmitPlan, ThreadId, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::{events, footprints, spawn, thread_attributes, threads};

/// Make the children of `parent_id` from `plan`, which the caller has
/// validated. See [`crate::ThreadDepStore::submit_plan`].
pub async fn submit(
    pool: &SqlitePool,
    parent_id: ThreadId,
    member_id: MemberId,
    plan: SubmitPlan,
) -> Result<PlanOutcome, StoreError> {
    let parent = parent_row(pool, parent_id).await?;
    hold_check(&parent, member_id, plan.lease_id)?;
    if let Some(outcome) = existing(pool, parent_id, &plan).await? {
        return Ok(outcome);
    }
    spawn_budget(pool, parent_id, &parent, plan.children.len() as i64).await?;

    let mut tx = pool.begin().await?;
    for child in &plan.children {
        if let Some(footprint) = &child.footprint {
            let allowed = sqlx::query(
                "SELECT 1 AS hit FROM maidan_egress_targets
                 WHERE workspace_id = ? AND surface = 'github_branch'
                   AND lower(substr(selector, 1, length(?) + 1)) = lower(?) || '@'
                 LIMIT 1",
            )
            .bind(parent.workspace_id)
            .bind(&footprint.repo)
            .bind(&footprint.repo)
            .fetch_optional(&mut *tx)
            .await?;
            if allowed.is_none() {
                return Err(StoreError::InvalidInput(format!(
                    "child `{}`: repository {} is not on this workspace's github_branch egress allowlist",
                    child.key, footprint.repo
                )));
            }
        }
    }
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_thread_plans (parent_thread_id, lease_id, submitted_by, created_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(parent_id.0)
    .bind(plan.lease_id.0)
    .bind(member_id.0)
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await?;

    let mut children = Vec::with_capacity(plan.children.len());
    let mut stored = Vec::with_capacity(plan.children.len());
    for (position, child) in plan.children.iter().enumerate() {
        let id = Uuid::now_v7();
        let row = sqlx::query(
            "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title, description, created_at, updated_at, created_by)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, created_by",
        )
        .bind(id)
        .bind(parent.channel_id)
        .bind(parent_id.0)
        .bind(&child.title)
        .bind(child.description.as_deref())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(member_id.0)
        .fetch_one(&mut *tx)
        .await?;
        let thread = threads::row_to_thread(&row)?;
        if let Some(footprint) = &child.footprint {
            footprints::insert_items_on(&mut tx, thread.id, parent.workspace_id, footprint).await?;
        }
        if let Some(attributes) = &child.attributes {
            thread_attributes::insert_on(&mut tx, thread.id, attributes).await?;
        }
        sqlx::query(
            "INSERT INTO maidan_thread_plan_children (parent_thread_id, child_key, child_thread_id, position)
             VALUES (?, ?, ?, ?)",
        )
        .bind(parent_id.0)
        .bind(&child.key)
        .bind(thread.id.0)
        .bind(position as i64)
        .execute(&mut *tx)
        .await?;
        stored.push(
            events::append_in_tx(
                &mut tx,
                &Event::ThreadCreated {
                    occurred_at: now,
                    workspace_id: WorkspaceId(parent.workspace_id),
                    channel_id: thread.channel_id,
                    thread: thread.clone(),
                },
            )
            .await?,
        );
        children.push(PlannedChild {
            key: child.key.clone(),
            thread_id: thread.id,
        });
    }
    for child in &plan.children {
        let Some(this) = children.iter().find(|c| c.key == child.key) else {
            continue;
        };
        for dep in &child.depends_on {
            let Some(on) = children.iter().find(|c| &c.key == dep) else {
                continue;
            };
            sqlx::query(
                "INSERT INTO maidan_thread_dependencies (thread_id, depends_on_thread_id, created_at)
                 VALUES (?, ?, ?)",
            )
            .bind(this.thread_id.0)
            .bind(on.thread_id.0)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(PlanOutcome::Created {
        children,
        events: stored,
    })
}

/// The progress of `parent_id`'s plan, or `None` when it has no children
/// from a plan.
pub async fn state(
    pool: &SqlitePool,
    parent_id: ThreadId,
) -> Result<Option<PlanState>, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS total,
                COALESCE(SUM(CASE WHEN t.state IN ('closed', 'archived') THEN 1 ELSE 0 END), 0) AS closed,
                COALESCE(SUM(CASE WHEN t.state NOT IN ('closed', 'archived') THEN 1 ELSE 0 END), 0) AS open,
                COALESCE(SUM(CASE WHEN t.state NOT IN ('closed', 'archived')
                                   AND EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = t.id)
                              THEN 1 ELSE 0 END), 0) AS blocked
         FROM maidan_thread_plan_children c
         JOIN maidan_threads t ON t.id = c.child_thread_id
         WHERE c.parent_thread_id = ?",
    )
    .bind(parent_id.0)
    .fetch_one(pool)
    .await?;
    let total: i64 = row.get("total");
    Ok((total > 0).then(|| PlanState {
        total,
        closed: row.get("closed"),
        open: row.get("open"),
        blocked: row.get("blocked"),
    }))
}

struct Parent {
    channel_id: Uuid,
    workspace_id: Uuid,
    assignee_id: Option<Uuid>,
    claim_lease_id: Option<Uuid>,
    assignment_expires_at: Option<DateTime<Utc>>,
    kind: Option<String>,
}

async fn parent_row(pool: &SqlitePool, parent_id: ThreadId) -> Result<Parent, StoreError> {
    let row = sqlx::query(
        "SELECT t.channel_id, c.workspace_id, t.assignee_id, t.claim_lease_id,
                t.assignment_expires_at, a.kind
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         LEFT JOIN maidan_thread_attributes a ON a.thread_id = t.id
         WHERE t.id = ? AND t.tombstoned_at IS NULL",
    )
    .bind(parent_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(Parent {
        channel_id: row.get("channel_id"),
        workspace_id: row.get("workspace_id"),
        assignee_id: row.get("assignee_id"),
        claim_lease_id: row.get("claim_lease_id"),
        assignment_expires_at: row.get("assignment_expires_at"),
        kind: row.get("kind"),
    })
}

/// Only a plan, and only from the member holding its live claim under
/// `lease_id`: a planner plans the task it claimed.
fn hold_check(
    parent: &Parent,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<(), StoreError> {
    if parent.kind.as_deref() != Some("plan") {
        return Err(StoreError::Conflict(
            "this thread is not a plan: only a thread with attributes.kind = plan takes children"
                .into(),
        ));
    }
    let live = parent
        .assignment_expires_at
        .is_none_or(|expires| expires > Utc::now());
    if parent.assignee_id != Some(member_id.0) || parent.claim_lease_id != Some(lease_id.0) || !live
    {
        return Err(StoreError::Conflict(
            "only the member holding this plan's live claim, under that claim's lease_id, can submit its children"
                .into(),
        ));
    }
    Ok(())
}

/// What an earlier plan on this parent means for this one: the same lease
/// with the same keys replays it, anything else conflicts with it.
async fn existing(
    pool: &SqlitePool,
    parent_id: ThreadId,
    plan: &SubmitPlan,
) -> Result<Option<PlanOutcome>, StoreError> {
    let Some(lease) = sqlx::query_scalar::<_, Uuid>(
        "SELECT lease_id FROM maidan_thread_plans WHERE parent_thread_id = ?",
    )
    .bind(parent_id.0)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let children: Vec<PlannedChild> = sqlx::query(
        "SELECT child_key, child_thread_id FROM maidan_thread_plan_children
         WHERE parent_thread_id = ? ORDER BY position",
    )
    .bind(parent_id.0)
    .fetch_all(pool)
    .await?
    .iter()
    .map(|row| PlannedChild {
        key: row.get("child_key"),
        thread_id: ThreadId(row.get("child_thread_id")),
    })
    .collect();
    let same_keys = children.len() == plan.children.len()
        && children
            .iter()
            .all(|c| plan.key_set().contains(c.key.as_str()));
    Ok(Some(if lease == plan.lease_id.0 && same_keys {
        PlanOutcome::Replayed(children)
    } else {
        PlanOutcome::Conflict(children)
    }))
}

/// The workspace's spawn budget, counted for the whole plan: a plan that
/// would take the parent past `max_children`, or nest past `max_depth`,
/// creates nothing.
async fn spawn_budget(
    pool: &SqlitePool,
    parent_id: ThreadId,
    parent: &Parent,
    adding: i64,
) -> Result<(), StoreError> {
    let workspace_id = WorkspaceId(parent.workspace_id);
    let Some(budget) = spawn::get_budget(pool, workspace_id).await? else {
        return Ok(());
    };
    let refuse = |axis, limit, observed| {
        Err(StoreError::spawn_rejected(SpawnDenial {
            workspace_id,
            channel_id: maidan_types::ChannelId(parent.channel_id),
            thread_id: parent_id,
            axis,
            limit,
            observed,
        }))
    };
    if let Some(max_children) = budget.max_children {
        let children = spawn::count_active_children(pool, parent_id).await?;
        if children + adding > max_children {
            return refuse(SpawnAxis::Children, max_children, children + adding);
        }
    }
    if let Some(max_depth) = budget.max_depth {
        let depth = spawn::thread_depth(pool, parent_id).await?;
        if depth >= max_depth {
            return refuse(SpawnAxis::Depth, max_depth, depth);
        }
    }
    Ok(())
}
