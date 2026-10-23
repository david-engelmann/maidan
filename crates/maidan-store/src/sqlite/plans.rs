//! Planner fan-out (#1421): a plan parent's children, made in one transaction
//! by the planner holding the parent's claim. SQLite twin of the Postgres
//! module.

use chrono::{DateTime, Utc};
use maidan_fsm::ThreadAction;
use maidan_types::{
    AmendOutcome, AmendPlan, AmendRefusal, BlockedReason, CancelAuthority, CancelOutcome,
    ClaimLeaseId, ContentKeyring, Event, ExistingPlanChild, MemberId, PlanAmendment,
    PlanChild as PlanChildSpec, PlanOutcome, PlanState, PlannedChild, SpawnAxis, SpawnDenial,
    StoredEvent, SubmitPlan, ThreadId, ThreadState, WorkspaceId, PLAN_MAX_AMENDMENTS,
};
use sqlx::{Acquire, Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::plan_notices;
use crate::sqlite::{
    events, footprints, messages, spawn, thread_attributes, thread_transitions, threads,
};

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
    check_repos_in_tx(&mut tx, parent.workspace_id, &plan.children).await?;
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

    let (children, stored) = insert_children_in_tx(
        &mut tx,
        &parent,
        parent_id,
        member_id,
        &plan.children,
        0,
        now,
    )
    .await?;
    for child in &plan.children {
        let Some(this) = children.iter().find(|c| c.key == child.key) else {
            continue;
        };
        for dep in &child.depends_on {
            let Some(on) = children.iter().find(|c| &c.key == dep) else {
                continue;
            };
            insert_dependency_in_tx(&mut tx, this.thread_id, on.thread_id, now).await?;
        }
    }
    // Held for review: each child is blocked, so `claim_next` skips it and the
    // board shows why, until a Maintainer releases the plan.
    if plan.needs_hold() {
        hold_in_tx(&mut tx, &children, member_id).await?;
    }
    tx.commit().await?;
    Ok(PlanOutcome::Created {
        children,
        events: stored,
    })
}

/// Every child's repository must be on the workspace's `github_branch`
/// egress allowlist (read only).
async fn check_repos_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    workspace_id: Uuid,
    children: &[PlanChildSpec],
) -> Result<(), StoreError> {
    for child in children {
        if let Some(footprint) = &child.footprint {
            let allowed = sqlx::query(
                "SELECT 1 AS hit FROM maidan_egress_targets
                 WHERE workspace_id = ? AND surface = 'github_branch'
                   AND lower(substr(selector, 1, length(?) + 1)) = lower(?) || '@'
                 LIMIT 1",
            )
            .bind(workspace_id)
            .bind(&footprint.repo)
            .bind(&footprint.repo)
            .fetch_optional(&mut **tx)
            .await?;
            if allowed.is_none() {
                return Err(StoreError::InvalidInput(format!(
                    "child `{}`: repository {} is not on this workspace's github_branch egress allowlist",
                    child.key, footprint.repo
                )));
            }
        }
    }
    Ok(())
}

/// Make `children_in` as threads under `parent_id`, each with its footprint,
/// attributes and plan row, from `first_position` on. Returns them by key,
/// with their `ThreadCreated` events.
async fn insert_children_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    parent: &Parent,
    parent_id: ThreadId,
    member_id: MemberId,
    children_in: &[PlanChildSpec],
    first_position: i64,
    now: DateTime<Utc>,
) -> Result<(Vec<PlannedChild>, Vec<StoredEvent>), StoreError> {
    let mut children = Vec::with_capacity(children_in.len());
    let mut stored = Vec::with_capacity(children_in.len());
    for (position, child) in children_in.iter().enumerate() {
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
        .fetch_one(&mut **tx)
        .await?;
        let thread = threads::row_to_thread(&row)?;
        if let Some(footprint) = &child.footprint {
            footprints::insert_items_on(tx, thread.id, parent.workspace_id, footprint).await?;
        }
        if let Some(attributes) = &child.attributes {
            thread_attributes::insert_on(tx, thread.id, attributes).await?;
        }
        thread_attributes::apply_default_engine_on(tx, thread.id).await?;
        sqlx::query(
            "INSERT INTO maidan_thread_plan_children (parent_thread_id, child_key, child_thread_id, position)
             VALUES (?, ?, ?, ?)",
        )
        .bind(parent_id.0)
        .bind(&child.key)
        .bind(thread.id.0)
        .bind(first_position + position as i64)
        .execute(&mut **tx)
        .await?;
        stored.push(
            events::append_in_tx(
                tx,
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
    Ok((children, stored))
}

async fn insert_dependency_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread: ThreadId,
    on: ThreadId,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_dependencies (thread_id, depends_on_thread_id, created_at)
         VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(thread.0)
    .bind(on.0)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Block each child with the plan hold, so `claim_next` skips it and the
/// board shows why, until a Maintainer releases the plan.
async fn hold_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    children: &[PlannedChild],
    member_id: MemberId,
) -> Result<(), StoreError> {
    for child in children {
        sqlx::query(
            "INSERT INTO maidan_thread_blocks (thread_id, reason, set_by, set_at, note)
             VALUES (?1, 'human', ?2, ?3, ?4)",
        )
        .bind(child.thread_id.0)
        .bind(member_id.0)
        .bind(Utc::now().to_rfc3339())
        .bind(maidan_types::PLAN_HOLD_NOTE)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
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

/// Whether `thread_id` is a child of a plan. A plan's children progress on
/// their own while the plan stays open under its planner's claim, so the
/// transition path lets them move ahead of their parent.
pub(crate) async fn is_child_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM maidan_thread_plan_children WHERE child_thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?
        > 0)
}

/// What a close owes the plans around the closed thread, in the closing
/// transaction (#1421). A child that landed closes its parent once every
/// child has landed, and that close repeats up through nested plans. A child
/// that closed without landing blocks its open dependents and tells its
/// parent once. Returns the events to publish.
pub(crate) async fn after_close_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    keys: &ContentKeyring,
    closed: ThreadId,
    actor: MemberId,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut stored = Vec::new();
    // A loop, not recursion: a closed parent may itself be a plan child.
    let mut next = Some(closed);
    while let Some(child) = next.take() {
        let Some(parent) = plan_parent_in_tx(tx, child).await? else {
            break;
        };
        if landing_in_tx(tx, child).await?.is_some() {
            let (events, parent_closed) =
                close_parent_in_tx(tx, keys, parent, child, actor).await?;
            stored.extend(events);
            if parent_closed {
                next = Some(parent);
            }
        } else {
            stored.extend(unlanded_in_tx(tx, keys, parent, child, actor).await?);
        }
    }
    Ok(stored)
}

async fn plan_parent_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    child: ThreadId,
) -> Result<Option<ThreadId>, StoreError> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT parent_thread_id FROM maidan_thread_plan_children WHERE child_thread_id = ?",
    )
    .bind(child.0)
    .fetch_optional(&mut **tx)
    .await?
    .map(ThreadId))
}

/// The pull request a thread landed with, from its latest `ThreadLanded`.
/// Landing is what that event records, so a thread closed by hand or
/// cancelled has none.
async fn landing_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<Option<(String, i64)>, StoreError> {
    let payload = sqlx::query_scalar::<_, sqlx::types::Json<serde_json::Value>>(
        "SELECT payload FROM maidan_events WHERE thread_id = ? AND kind = 'thread_landed'
         ORDER BY id DESC LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(payload.and_then(|p| plan_notices::landing_from_payload(&p.0)))
}

/// Close `parent` when every child has landed, through the transition path
/// so every close gate applies. A refusal rolls back to a savepoint and
/// leaves one message instead: the child's landing stands either way.
async fn close_parent_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    keys: &ContentKeyring,
    parent: ThreadId,
    completing: ThreadId,
    actor: MemberId,
) -> Result<(Vec<StoredEvent>, bool), StoreError> {
    let state: String = sqlx::query_scalar("SELECT state FROM maidan_threads WHERE id = ?")
        .bind(parent.0)
        .fetch_one(&mut **tx)
        .await?;
    if state != ThreadState::Open.as_str() && state != ThreadState::InReview.as_str() {
        return Ok((Vec::new(), false));
    }
    let rows = sqlx::query(
        "SELECT t.id, t.title, t.state FROM maidan_thread_plan_children c
         JOIN maidan_threads t ON t.id = c.child_thread_id
         WHERE c.parent_thread_id = ? ORDER BY c.position",
    )
    .bind(parent.0)
    .fetch_all(&mut **tx)
    .await?;
    let mut children = Vec::with_capacity(rows.len());
    for row in &rows {
        let state: String = row.get("state");
        if state == ThreadState::Open.as_str() || state == ThreadState::InReview.as_str() {
            return Ok((Vec::new(), false));
        }
        let thread_id = ThreadId(row.get("id"));
        let Some(landed) = landing_in_tx(tx, thread_id).await? else {
            // That child's close already told the parent.
            return Ok((Vec::new(), false));
        };
        children.push(plan_notices::ChildLine {
            thread_id,
            title: row.get("title"),
            landed: Some(landed),
        });
    }
    let Some((repo, pr_number)) = landing_in_tx(tx, completing).await? else {
        return Ok((Vec::new(), false));
    };

    let mut savepoint = tx.begin().await?;
    match close_through_gates(
        &mut savepoint,
        parent,
        actor,
        state == ThreadState::Open.as_str(),
    )
    .await
    {
        Ok(mut stored) => {
            let (workspace_id, channel_id) =
                events::thread_scope_in_tx(&mut savepoint, parent).await?;
            let landed = Event::ThreadLanded {
                occurred_at: Utc::now(),
                workspace_id,
                channel_id,
                thread_id: parent,
                repo,
                pr_number,
                merged_by: None,
                merge_commit_sha: None,
                title: None,
            };
            stored.push(events::append_in_tx(&mut savepoint, &landed).await?);
            stored.push(
                post_notice_in_tx(
                    &mut savepoint,
                    keys,
                    parent,
                    actor,
                    plan_notices::landed_body(&children),
                    plan_notices::landed_metadata(),
                )
                .await?,
            );
            savepoint.commit().await?;
            Ok((stored, true))
        }
        Err(StoreError::Conflict(reason)) => {
            savepoint.rollback().await?;
            let posted = post_notice_in_tx(
                tx,
                keys,
                parent,
                actor,
                plan_notices::refused_body(&reason),
                plan_notices::refused_metadata(&reason),
            )
            .await?;
            Ok((vec![posted], false))
        }
        Err(other) => Err(other),
    }
}

async fn close_through_gates(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    parent: ThreadId,
    actor: MemberId,
    from_open: bool,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut stored = Vec::new();
    // No edge runs from open to closed: the plan passes through review, and
    // the review and close gates apply to it as to any thread.
    if from_open {
        let review =
            thread_transitions::transition_in_tx(tx, parent, actor, ThreadAction::StartReview)
                .await?;
        stored.push(thread_transitions::state_changed_in_tx(tx, actor, &review).await?);
    }
    let closed =
        thread_transitions::transition_in_tx(tx, parent, actor, ThreadAction::Close).await?;
    stored.push(thread_transitions::state_changed_in_tx(tx, actor, &closed).await?);
    Ok(stored)
}

/// Block each open dependent of `child`, which closed without landing, and
/// tell `parent` once.
async fn unlanded_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    keys: &ContentKeyring,
    parent: ThreadId,
    child: ThreadId,
    actor: MemberId,
) -> Result<Vec<StoredEvent>, StoreError> {
    let title: String = sqlx::query_scalar("SELECT title FROM maidan_threads WHERE id = ?")
        .bind(child.0)
        .fetch_one(&mut **tx)
        .await?;
    let dependents = sqlx::query_scalar::<_, Uuid>(
        "SELECT t.id FROM maidan_thread_dependencies d
         JOIN maidan_threads t ON t.id = d.thread_id
         WHERE d.depends_on_thread_id = ? AND t.state IN ('open', 'in_review')
           AND t.tombstoned_at IS NULL
         ORDER BY t.id",
    )
    .bind(child.0)
    .fetch_all(&mut **tx)
    .await?;
    let mut stored = Vec::new();
    let note = plan_notices::blocked_note(&title, child);
    for dependent in dependents.into_iter().map(ThreadId) {
        // A block a person set stays as it is: it already keeps the thread
        // off the queue, and its reason is theirs.
        let inserted = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO maidan_thread_blocks (thread_id, reason, set_by, set_at, note)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (thread_id) DO NOTHING
             RETURNING thread_id",
        )
        .bind(dependent.0)
        .bind(BlockedReason::Dag.as_str())
        .bind(actor.0)
        .bind(Utc::now().to_rfc3339())
        .bind(&note)
        .fetch_optional(&mut **tx)
        .await?;
        if inserted.is_none() {
            continue;
        }
        let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, dependent).await?;
        let blocked = Event::ThreadBlocked {
            occurred_at: Utc::now(),
            workspace_id,
            channel_id,
            thread_id: dependent,
            reason: BlockedReason::Dag,
            set_by: actor,
            note: Some(note.clone()),
        };
        stored.push(events::append_in_tx(tx, &blocked).await?);
        stored.push(
            post_notice_in_tx(
                tx,
                keys,
                dependent,
                actor,
                plan_notices::blocked_body(&title, child),
                plan_notices::blocked_metadata(child),
            )
            .await?,
        );
    }
    let line = plan_notices::ChildLine {
        thread_id: child,
        title,
        landed: None,
    };
    stored.push(
        post_notice_in_tx(
            tx,
            keys,
            parent,
            actor,
            plan_notices::unlanded_body(&line),
            plan_notices::unlanded_metadata(child),
        )
        .await?,
    );
    Ok(stored)
}

/// Post a system message on `thread_id`, sealed like any other message.
async fn post_notice_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    keys: &ContentKeyring,
    thread_id: ThreadId,
    author: MemberId,
    body: String,
    metadata: serde_json::Value,
) -> Result<StoredEvent, StoreError> {
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_messages (id, thread_id, author_id, body, metadata, content, posted_at)
         VALUES (?, ?, ?, ?, ?, NULL, ?)
         RETURNING id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at",
    )
    .bind(Uuid::now_v7())
    .bind(thread_id.0)
    .bind(author.0)
    .bind(body)
    .bind(serde_json::to_string(&metadata)?)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?;
    let message = messages::row_to_message(&row)?;
    let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, thread_id).await?;
    let posted = Event::MessagePosted {
        occurred_at: now,
        workspace_id,
        channel_id,
        thread_id,
        dm_conversation_id: None,
        message,
        sealed: None,
    };
    events::append_with_keys_in_tx(tx, Some(keys), &posted, None).await
}

/// Release a held plan: clear the hold on each of `parent_id`'s children
/// (only the plan hold; any other block stays), with its audit row in the same
/// transaction. Returns the children released; empty when none was held.
pub async fn release(
    pool: &SqlitePool,
    parent_id: ThreadId,
    audit: maidan_types::NewAuditEvent,
) -> Result<Vec<ThreadId>, StoreError> {
    let mut tx = pool.begin().await?;
    let released: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM maidan_thread_blocks
         WHERE note = ?2 AND thread_id IN (
             SELECT child_thread_id FROM maidan_thread_plan_children WHERE parent_thread_id = ?1)
         RETURNING thread_id",
    )
    .bind(parent_id.0)
    .bind(maidan_types::PLAN_HOLD_NOTE)
    .fetch_all(&mut *tx)
    .await?;
    if !released.is_empty() {
        super::audit::append_scoped(
            &mut tx,
            crate::audit_scope::AuditOwner::Thread(parent_id),
            audit,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(released.into_iter().map(ThreadId).collect())
}

/// Amend `parent_id`'s plan in one transaction; see
/// [`crate::ThreadDepStore::amend_plan`]. `amend` is checked by the caller.
pub async fn amend(
    pool: &SqlitePool,
    keys: &ContentKeyring,
    parent_id: ThreadId,
    member_id: MemberId,
    amend: AmendPlan,
) -> Result<AmendOutcome, StoreError> {
    let parent = parent_row(pool, parent_id).await?;
    hold_check(&parent, member_id, amend.lease_id)?;
    let net_adding = amend.add.len() as i64 - amend.cancel.len() as i64;
    if net_adding > 0 {
        spawn_budget(pool, parent_id, &parent, net_adding).await?;
    }

    let mut tx = pool.begin().await?;
    let amendments: i64 =
        sqlx::query_scalar("SELECT amendments FROM maidan_thread_plans WHERE parent_thread_id = ?")
            .bind(parent_id.0)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                StoreError::Conflict("this plan has no children yet: submit it first".into())
            })?;
    if amendments >= PLAN_MAX_AMENDMENTS {
        return Err(StoreError::Conflict(format!(
            "this plan has been amended {amendments} times, the most it takes"
        )));
    }
    let rows = sqlx::query(
        "SELECT c.child_key, c.child_thread_id, c.position, t.state, t.assignee_id
         FROM maidan_thread_plan_children c
         JOIN maidan_threads t ON t.id = c.child_thread_id
         WHERE c.parent_thread_id = ? ORDER BY c.position",
    )
    .bind(parent_id.0)
    .fetch_all(&mut *tx)
    .await?;
    let edges = sqlx::query(
        "SELECT d.thread_id, d.depends_on_thread_id
         FROM maidan_thread_dependencies d
         JOIN maidan_thread_plan_children a ON a.child_thread_id = d.thread_id
         JOIN maidan_thread_plan_children b ON b.child_thread_id = d.depends_on_thread_id
         WHERE a.parent_thread_id = ?1 AND b.parent_thread_id = ?1",
    )
    .bind(parent_id.0)
    .fetch_all(&mut *tx)
    .await?;
    let mut next_position = 0_i64;
    let mut existing: Vec<ExistingPlanChild> = Vec::with_capacity(rows.len());
    for row in &rows {
        next_position = next_position.max(row.get::<i64, _>("position") + 1);
        let state: String = row.get("state");
        existing.push(ExistingPlanChild {
            key: row.get("child_key"),
            thread_id: ThreadId(row.get("child_thread_id")),
            unclaimed: state == "open" && row.get::<Option<Uuid>, _>("assignee_id").is_none(),
            depends_on: Vec::new(),
        });
    }
    for edge in &edges {
        let from = ThreadId(edge.get("thread_id"));
        let on = ThreadId(edge.get("depends_on_thread_id"));
        let on_key = existing
            .iter()
            .find(|c| c.thread_id == on)
            .map(|c| c.key.clone());
        if let (Some(child), Some(on_key)) =
            (existing.iter_mut().find(|c| c.thread_id == from), on_key)
        {
            child.depends_on.push(on_key);
        }
    }
    let steps = amend.steps(&existing).map_err(refusal)?;
    check_repos_in_tx(&mut tx, parent.workspace_id, &steps.add).await?;
    let held = amend.adds_need_hold()
        || sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM maidan_thread_blocks b
             JOIN maidan_thread_plan_children c ON c.child_thread_id = b.thread_id
             WHERE c.parent_thread_id = ? AND b.note = ?",
        )
        .bind(parent_id.0)
        .bind(maidan_types::PLAN_HOLD_NOTE)
        .fetch_one(&mut *tx)
        .await?
            > 0;

    let now = Utc::now();
    let reason = amend
        .reason
        .as_deref()
        .unwrap_or(maidan_types::PLAN_AMEND_CANCEL_REASON);
    let mut stored = Vec::new();
    // Out of the plan first, so the cancel's close does not block the
    // children that waited for it: the amendment rewires them instead.
    for child in &steps.cancel {
        sqlx::query("DELETE FROM maidan_thread_plan_children WHERE child_thread_id = ?")
            .bind(child.thread_id.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM maidan_thread_dependencies WHERE depends_on_thread_id = ?")
            .bind(child.thread_id.0)
            .execute(&mut *tx)
            .await?;
        match super::cancel::cancel_in_tx(
            &mut tx,
            keys,
            child.thread_id,
            member_id,
            reason,
            CancelAuthority::Transition,
        )
        .await?
        {
            CancelOutcome::Cancelled { events, .. } => stored.extend(events),
            _ => {
                return Err(StoreError::Conflict(format!(
                    "child `{}` finished while the amendment ran",
                    child.key
                )))
            }
        }
    }
    let (added, created) = insert_children_in_tx(
        &mut tx,
        &parent,
        parent_id,
        member_id,
        &steps.add,
        next_position,
        now,
    )
    .await?;
    stored.extend(created);
    let mut thread_of: std::collections::HashMap<String, ThreadId> = existing
        .iter()
        .filter(|c| !steps.cancel.iter().any(|x| x.thread_id == c.thread_id))
        .map(|c| (c.key.clone(), c.thread_id))
        .collect();
    thread_of.extend(added.iter().map(|c| (c.key.clone(), c.thread_id)));
    let wire = |key: &str| {
        thread_of.get(key).copied().ok_or_else(|| {
            StoreError::InvalidInput(format!("`{key}` is not a key the amended plan keeps"))
        })
    };
    for child in &steps.add {
        let this = wire(&child.key)?;
        for dep in &child.depends_on {
            insert_dependency_in_tx(&mut tx, this, wire(dep)?, now).await?;
        }
    }
    for (key, deps) in &steps.rewrite {
        let this = wire(key)?;
        sqlx::query(
            "DELETE FROM maidan_thread_dependencies
             WHERE thread_id = ?1 AND depends_on_thread_id IN (
                 SELECT child_thread_id FROM maidan_thread_plan_children WHERE parent_thread_id = ?2)",
        )
        .bind(this.0)
        .bind(parent_id.0)
        .execute(&mut *tx)
        .await?;
        for dep in deps {
            insert_dependency_in_tx(&mut tx, this, wire(dep)?, now).await?;
        }
    }
    if held {
        hold_in_tx(&mut tx, &added, member_id).await?;
    }
    sqlx::query(
        "UPDATE maidan_thread_plans SET amendments = amendments + 1 WHERE parent_thread_id = ?",
    )
    .bind(parent_id.0)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(AmendOutcome {
        amendment: PlanAmendment {
            cancelled: steps.cancel,
            added,
            rewired: steps.rewrite.into_iter().map(|(key, _)| key).collect(),
            amendments: amendments + 1,
            held,
        },
        events: stored,
    })
}

/// An amendment refused against the plan as it stands: a malformed one is a
/// 400, one that would change a claimed or finished child a 409.
fn refusal(refusal: AmendRefusal) -> StoreError {
    match refusal {
        AmendRefusal::Invalid(why) => StoreError::InvalidInput(why),
        AmendRefusal::Settled(why) => StoreError::Conflict(why),
    }
}
