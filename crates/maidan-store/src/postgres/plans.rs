//! Planner fan-out (#1421): a plan parent's children, made in one transaction
//! by the planner holding the parent's claim. The parent row is locked for
//! the transaction, so two submissions on one parent run one after the other.

use chrono::{DateTime, Utc};
use maidan_fsm::ThreadAction;
use maidan_types::{
    AmendOutcome, AmendPlan, AmendRefusal, BlockedReason, CancelAuthority, CancelOutcome,
    ClaimLeaseId, ContentKeyring, Event, ExistingPlanChild, MemberId, PlanAmendment,
    PlanChild as PlanChildSpec, PlanOutcome, PlanState, PlannedChild, SpawnAxis, SpawnDenial,
    StoredEvent, SubmitPlan, ThreadId, ThreadState, WorkspaceId, PLAN_MAX_AMENDMENTS,
};
use sqlx::{Acquire, PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::plan_notices;
use crate::postgres::{
    events, footprints, messages, spawn, thread_attributes, thread_transitions, threads,
};

/// Make the children of `parent_id` from `plan`, which the caller has
/// validated. See [`crate::ThreadDepStore::submit_plan`].
pub async fn submit(
    pool: &PgPool,
    parent_id: ThreadId,
    member_id: MemberId,
    plan: SubmitPlan,
) -> Result<PlanOutcome, StoreError> {
    let mut tx = pool.begin().await?;
    let parent = parent_row(&mut tx, parent_id).await?;
    hold_check(&parent, member_id, plan.lease_id)?;
    // A retry is settled before the budget is consulted: replaying what a
    // claim already made (or conflicting with it) must not fail because
    // the workspace has since filled its spawn budget.
    if let Some(outcome) = existing(&mut tx, parent_id, &plan).await? {
        return Ok(outcome);
    }
    spawn_budget_precheck(pool, parent_id, plan.children.len() as i64).await?;
    check_repos_in_tx(&mut tx, parent.workspace_id, &plan.children).await?;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_thread_plans (parent_thread_id, lease_id, submitted_by, created_at)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(parent_id.0)
    .bind(plan.lease_id.0)
    .bind(member_id.0)
    .bind(now)
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
            insert_dependency_in_tx(&mut tx, this.thread_id, on.thread_id).await?;
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workspace_id: Uuid,
    children: &[PlanChildSpec],
) -> Result<(), StoreError> {
    for child in children {
        if let Some(footprint) = &child.footprint {
            let allowed = sqlx::query(
                "SELECT 1 AS hit FROM maidan_egress_targets
                 WHERE workspace_id = $1 AND surface = 'github_branch'
                   AND lower(substr(selector, 1, length($2) + 1)) = lower($2) || '@'
                 LIMIT 1",
            )
            .bind(workspace_id)
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
            "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title, description, created_by)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, created_by",
        )
        .bind(id)
        .bind(parent.channel_id)
        .bind(parent_id.0)
        .bind(&child.title)
        .bind(child.description.as_deref())
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
             VALUES ($1, $2, $3, $4)",
        )
        .bind(parent_id.0)
        .bind(&child.key)
        .bind(thread.id.0)
        .bind((first_position + position as i64) as i32)
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread: ThreadId,
    on: ThreadId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_dependencies (thread_id, depends_on_thread_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(thread.0)
    .bind(on.0)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Block each child with the plan hold, so `claim_next` skips it and the
/// board shows why, until a Maintainer releases the plan.
async fn hold_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    children: &[PlannedChild],
    member_id: MemberId,
) -> Result<(), StoreError> {
    for child in children {
        sqlx::query(
            "INSERT INTO maidan_thread_blocks (thread_id, reason, set_by, note)
             VALUES ($1, 'human', $2, $3)",
        )
        .bind(child.thread_id.0)
        .bind(member_id.0)
        .bind(maidan_types::PLAN_HOLD_NOTE)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// The progress of `parent_id`'s plan, or `None` when it has no children
/// from a plan.
pub async fn state(pool: &PgPool, parent_id: ThreadId) -> Result<Option<PlanState>, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS total,
                COALESCE(SUM(CASE WHEN t.state IN ('closed', 'archived') THEN 1 ELSE 0 END), 0)::BIGINT AS closed,
                COALESCE(SUM(CASE WHEN t.state NOT IN ('closed', 'archived') THEN 1 ELSE 0 END), 0)::BIGINT AS open,
                COALESCE(SUM(CASE WHEN t.state NOT IN ('closed', 'archived')
                                   AND EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = t.id)
                              THEN 1 ELSE 0 END), 0)::BIGINT AS blocked
         FROM maidan_thread_plan_children c
         JOIN maidan_threads t ON t.id = c.child_thread_id
         WHERE c.parent_thread_id = $1",
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

async fn parent_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    parent_id: ThreadId,
) -> Result<Parent, StoreError> {
    let row = sqlx::query(
        "SELECT t.channel_id, c.workspace_id, t.assignee_id, t.claim_lease_id,
                t.assignment_expires_at, a.kind
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         LEFT JOIN maidan_thread_attributes a ON a.thread_id = t.id
         WHERE t.id = $1 AND t.tombstoned_at IS NULL
         FOR UPDATE OF t",
    )
    .bind(parent_id.0)
    .fetch_optional(&mut **tx)
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    parent_id: ThreadId,
    plan: &SubmitPlan,
) -> Result<Option<PlanOutcome>, StoreError> {
    let Some(lease) = sqlx::query_scalar::<_, Uuid>(
        "SELECT lease_id FROM maidan_thread_plans WHERE parent_thread_id = $1",
    )
    .bind(parent_id.0)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    let children: Vec<PlannedChild> = sqlx::query(
        "SELECT child_key, child_thread_id FROM maidan_thread_plan_children
         WHERE parent_thread_id = $1 ORDER BY position",
    )
    .bind(parent_id.0)
    .fetch_all(&mut **tx)
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
    if lease != plan.lease_id.0 || !same_keys {
        return Ok(Some(PlanOutcome::Conflict(children)));
    }
    // The stored plan's hold, from the blocks standing on its children:
    // the retry's own shape says nothing about the plan as it stands.
    let held = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM maidan_thread_blocks b
         JOIN maidan_thread_plan_children c ON c.child_thread_id = b.thread_id
         WHERE c.parent_thread_id = $1 AND b.note = $2",
    )
    .bind(parent_id.0)
    .bind(maidan_types::PLAN_HOLD_NOTE)
    .fetch_one(&mut **tx)
    .await?
        > 0;
    Ok(Some(PlanOutcome::Replayed { children, held }))
}

/// The workspace's spawn budget, counted for the whole plan: a plan that
/// would take the parent past `max_children`, or nest past `max_depth`,
/// creates nothing.
async fn spawn_budget_precheck(
    pool: &PgPool,
    parent_id: ThreadId,
    adding: i64,
) -> Result<(), StoreError> {
    let Some(row) = sqlx::query(
        "SELECT t.channel_id, c.workspace_id FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id WHERE t.id = $1",
    )
    .bind(parent_id.0)
    .fetch_optional(pool)
    .await?
    else {
        return Err(StoreError::NotFound);
    };
    let workspace_id = WorkspaceId(row.get("workspace_id"));
    let channel_id = maidan_types::ChannelId(row.get("channel_id"));
    let Some(budget) = spawn::get_budget(pool, workspace_id).await? else {
        return Ok(());
    };
    let refuse = |axis, limit, observed| {
        Err(StoreError::spawn_rejected(SpawnDenial {
            workspace_id,
            channel_id,
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM maidan_thread_plan_children WHERE child_thread_id = $1",
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    child: ThreadId,
) -> Result<Option<ThreadId>, StoreError> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT parent_thread_id FROM maidan_thread_plan_children WHERE child_thread_id = $1",
    )
    .bind(child.0)
    .fetch_optional(&mut **tx)
    .await?
    .map(ThreadId))
}

/// Whether a closed thread landed, and with what pull request. No single
/// record is always written, so the signal is hybrid: the sweep's settle
/// writes `ThreadLanded` for the merges it closes, but a thread whose
/// pull merged can be closed by hand before the sweep runs, and a thread
/// with no pull request at all (research, docs, manual work) lands by
/// its plain close. A change pull that closed unmerged is the one close
/// that is not a landing.
async fn landing_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
) -> Result<Option<plan_notices::Landing>, StoreError> {
    let payload = sqlx::query_scalar::<_, sqlx::types::Json<serde_json::Value>>(
        "SELECT payload FROM maidan_events WHERE thread_id = $1 AND kind = 'thread_landed'
         ORDER BY id DESC LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((repo, pr)) = payload.and_then(|p| plan_notices::landing_from_payload(&p.0)) {
        return Ok(Some(plan_notices::Landing::Pull(repo, pr)));
    }
    let merged = sqlx::query(
        "SELECT repo, pull_number FROM maidan_change_pulls
         WHERE thread_id = $1 AND merged_at IS NOT NULL
         ORDER BY merged_at DESC LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(row) = merged {
        return Ok(Some(plan_notices::Landing::Pull(
            row.get("repo"),
            row.get("pull_number"),
        )));
    }
    let pulls: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM maidan_change_pulls WHERE thread_id = $1")
            .bind(thread_id.0)
            .fetch_one(&mut **tx)
            .await?;
    Ok(if pulls == 0 {
        Some(plan_notices::Landing::Plain)
    } else {
        None
    })
}

/// Close `parent` when every child has landed, through the transition path
/// so every close gate applies. A refusal rolls back to a savepoint and
/// leaves one message instead: the child's landing stands either way.
async fn close_parent_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    keys: &ContentKeyring,
    parent: ThreadId,
    completing: ThreadId,
    actor: MemberId,
) -> Result<(Vec<StoredEvent>, bool), StoreError> {
    // Locked, so of two children landing at once the second sees the first
    // closed and closes the plan.
    let state: String =
        sqlx::query_scalar("SELECT state::text FROM maidan_threads WHERE id = $1 FOR UPDATE")
            .bind(parent.0)
            .fetch_one(&mut **tx)
            .await?;
    if state != ThreadState::Open.as_str() && state != ThreadState::InReview.as_str() {
        return Ok((Vec::new(), false));
    }
    let rows = sqlx::query(
        "SELECT t.id, t.title, t.state::text AS state FROM maidan_thread_plan_children c
         JOIN maidan_threads t ON t.id = c.child_thread_id
         WHERE c.parent_thread_id = $1 ORDER BY c.position",
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
        let Some(landing) = landing_in_tx(tx, thread_id).await? else {
            // That child's close already told the parent.
            return Ok((Vec::new(), false));
        };
        children.push(plan_notices::ChildLine {
            thread_id,
            title: row.get("title"),
            landed: landing.pull(),
        });
    }
    let Some(completing_landing) = landing_in_tx(tx, completing).await? else {
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
            // The plan itself lands with the completing child's pull
            // request; a plan finished by plain closes has none to name,
            // and its own parent reads the plain close instead.
            if let Some((repo, pr_number)) = completing_landing.pull() {
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
            }
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    keys: &ContentKeyring,
    parent: ThreadId,
    child: ThreadId,
    actor: MemberId,
) -> Result<Vec<StoredEvent>, StoreError> {
    let title: String = sqlx::query_scalar("SELECT title FROM maidan_threads WHERE id = $1")
        .bind(child.0)
        .fetch_one(&mut **tx)
        .await?;
    let dependents = sqlx::query_scalar::<_, Uuid>(
        "SELECT t.id FROM maidan_thread_dependencies d
         JOIN maidan_threads t ON t.id = d.thread_id
         WHERE d.depends_on_thread_id = $1 AND t.state IN ('open', 'in_review')
           AND t.tombstoned_at IS NULL
         ORDER BY t.id",
    )
    .bind(child.0)
    .fetch_all(&mut **tx)
    .await?;
    let mut stored = Vec::new();
    for dependent in dependents.into_iter().map(ThreadId) {
        stored.extend(dag_block_in_tx(tx, keys, dependent, child, &title, actor).await?);
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

/// Write the `dag` block on `dependent` for `dependency`, which closed
/// without landing, with its event and its notice. A block already
/// standing stays as it is: it already keeps the thread off the queue,
/// and its reason is its setter's. Returns the events, empty when a
/// block already stood.
async fn dag_block_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    keys: &ContentKeyring,
    dependent: ThreadId,
    dependency: ThreadId,
    dependency_title: &str,
    actor: MemberId,
) -> Result<Vec<StoredEvent>, StoreError> {
    let note = plan_notices::blocked_note(dependency_title, dependency);
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO maidan_thread_blocks (thread_id, reason, set_by, note)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (thread_id) DO NOTHING
         RETURNING thread_id",
    )
    .bind(dependent.0)
    .bind(BlockedReason::Dag.as_str())
    .bind(actor.0)
    .bind(&note)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(_) = inserted else {
        return Ok(Vec::new());
    };
    let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, dependent).await?;
    let blocked = Event::ThreadBlocked {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id: dependent,
        reason: BlockedReason::Dag,
        set_by: actor,
        note: Some(note),
    };
    Ok(vec![
        events::append_in_tx(tx, &blocked).await?,
        post_notice_in_tx(
            tx,
            keys,
            dependent,
            actor,
            plan_notices::blocked_body(dependency_title, dependency),
            plan_notices::blocked_metadata(dependency),
        )
        .await?,
    ])
}

/// Block each `(dependent, dependency)` pair whose dependency has already
/// closed without landing, with the block the close path writes. An
/// amendment uses this for the children it adds or rewires onto such a
/// dependency: the close ran before they depended on it, so nothing
/// blocked them, and `claim_next` would hand them out over work that
/// never merged. Returns the events and the children blocked.
async fn block_unlanded_dependents_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    keys: &ContentKeyring,
    pairs: &[(ThreadId, ThreadId)],
    actor: MemberId,
) -> Result<(Vec<StoredEvent>, Vec<ThreadId>), StoreError> {
    let mut stored = Vec::new();
    let mut blocked = Vec::new();
    for &(dependent, dependency) in pairs {
        let row =
            sqlx::query("SELECT state::text AS state, title FROM maidan_threads WHERE id = $1")
                .bind(dependency.0)
                .fetch_one(&mut **tx)
                .await?;
        let state: String = row.get("state");
        let settled =
            state == ThreadState::Closed.as_str() || state == ThreadState::Archived.as_str();
        if !settled || landing_in_tx(tx, dependency).await?.is_some() {
            continue;
        }
        let title: String = row.get("title");
        let events = dag_block_in_tx(tx, keys, dependent, dependency, &title, actor).await?;
        if !events.is_empty() {
            blocked.push(dependent);
            stored.extend(events);
        }
    }
    Ok((stored, blocked))
}

/// Post a system message on `thread_id`, sealed like any other message.
async fn post_notice_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    keys: &ContentKeyring,
    thread_id: ThreadId,
    author: MemberId,
    body: String,
    metadata: serde_json::Value,
) -> Result<StoredEvent, StoreError> {
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_messages (id, thread_id, author_id, body, metadata, content, posted_at)
         VALUES ($1, $2, $3, $4, $5, NULL, $6)
         RETURNING id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at",
    )
    .bind(Uuid::now_v7())
    .bind(thread_id.0)
    .bind(author.0)
    .bind(body)
    .bind(metadata)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?;
    let message = messages::row_to_message(&row);
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
    pool: &PgPool,
    parent_id: ThreadId,
    audit: maidan_types::NewAuditEvent,
) -> Result<Vec<ThreadId>, StoreError> {
    let mut tx = pool.begin().await?;
    let released: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM maidan_thread_blocks
         WHERE note = $2 AND thread_id IN (
             SELECT child_thread_id FROM maidan_thread_plan_children WHERE parent_thread_id = $1)
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
    pool: &PgPool,
    keys: &ContentKeyring,
    parent_id: ThreadId,
    member_id: MemberId,
    amend: AmendPlan,
) -> Result<AmendOutcome, StoreError> {
    let net_adding = amend.add.len() as i64 - amend.cancel.len() as i64;
    if net_adding > 0 {
        spawn_budget_precheck(pool, parent_id, net_adding).await?;
    }
    let mut tx = pool.begin().await?;
    // Locked for the transaction, so two amendments run one after the other
    // and the cap counts both.
    let parent = parent_row(&mut tx, parent_id).await?;
    hold_check(&parent, member_id, amend.lease_id)?;
    let amendments: i64 = sqlx::query_scalar(
        "SELECT amendments::BIGINT FROM maidan_thread_plans WHERE parent_thread_id = $1",
    )
    .bind(parent_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| StoreError::Conflict("this plan has no children yet: submit it first".into()))?;
    if amendments >= PLAN_MAX_AMENDMENTS {
        return Err(StoreError::Conflict(format!(
            "this plan has been amended {amendments} times, the most it takes"
        )));
    }
    let rows = sqlx::query(
        "SELECT c.child_key, c.child_thread_id, c.position, t.state, t.assignee_id
         FROM maidan_thread_plan_children c
         JOIN maidan_threads t ON t.id = c.child_thread_id
         WHERE c.parent_thread_id = $1 ORDER BY c.position",
    )
    .bind(parent_id.0)
    .fetch_all(&mut *tx)
    .await?;
    let edges = sqlx::query(
        "SELECT d.thread_id, d.depends_on_thread_id
         FROM maidan_thread_dependencies d
         JOIN maidan_thread_plan_children a ON a.child_thread_id = d.thread_id
         JOIN maidan_thread_plan_children b ON b.child_thread_id = d.depends_on_thread_id
         WHERE a.parent_thread_id = $1 AND b.parent_thread_id = $1",
    )
    .bind(parent_id.0)
    .fetch_all(&mut *tx)
    .await?;
    let mut next_position = 0_i64;
    let mut existing: Vec<ExistingPlanChild> = Vec::with_capacity(rows.len());
    for row in &rows {
        next_position = next_position.max(i64::from(row.get::<i32, _>("position")) + 1);
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
            "SELECT COUNT(*)::BIGINT FROM maidan_thread_blocks b
             JOIN maidan_thread_plan_children c ON c.child_thread_id = b.thread_id
             WHERE c.parent_thread_id = $1 AND b.note = $2",
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
        sqlx::query("DELETE FROM maidan_thread_plan_children WHERE child_thread_id = $1")
            .bind(child.thread_id.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM maidan_thread_dependencies WHERE depends_on_thread_id = $1")
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
            insert_dependency_in_tx(&mut tx, this, wire(dep)?).await?;
        }
    }
    for (key, deps) in &steps.rewrite {
        let this = wire(key)?;
        sqlx::query(
            "DELETE FROM maidan_thread_dependencies
             WHERE thread_id = $1 AND depends_on_thread_id IN (
                 SELECT child_thread_id FROM maidan_thread_plan_children WHERE parent_thread_id = $2)",
        )
        .bind(this.0)
        .bind(parent_id.0)
        .execute(&mut *tx)
        .await?;
        for dep in deps {
            insert_dependency_in_tx(&mut tx, this, wire(dep)?).await?;
        }
    }
    // A child this amendment points at a dependency that already closed
    // without landing takes the block the close path writes: the close
    // ran before the child depended on it, so nothing blocked it then.
    let mut pairs: Vec<(ThreadId, ThreadId)> = Vec::new();
    for child in &steps.add {
        let this = wire(&child.key)?;
        for dep in &child.depends_on {
            pairs.push((this, wire(dep)?));
        }
    }
    for (key, deps) in &steps.rewrite {
        let this = wire(key)?;
        for dep in deps {
            pairs.push((this, wire(dep)?));
        }
    }
    let (block_events, dag_blocked) =
        block_unlanded_dependents_in_tx(&mut tx, keys, &pairs, member_id).await?;
    stored.extend(block_events);
    if held {
        // A child the dag block already holds keeps that block: a thread
        // carries one block, and a release must not free it.
        let unblocked: Vec<PlannedChild> = added
            .iter()
            .filter(|c| !dag_blocked.contains(&c.thread_id))
            .cloned()
            .collect();
        hold_in_tx(&mut tx, &unblocked, member_id).await?;
    }
    sqlx::query(
        "UPDATE maidan_thread_plans SET amendments = amendments + 1 WHERE parent_thread_id = $1",
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
