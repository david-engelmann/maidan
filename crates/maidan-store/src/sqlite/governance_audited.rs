//! D-A for governance and membership: who is frozen, who is in a channel, what
//! a thread's close-gates require, which skills confer approval authority,
//! where results may egress, and which app installations stand. Each change
//! writes its audit row in its own transaction, so a failed write aborts it.
//! Where a call can remove nothing, it records nothing.

use maidan_types::{
    AllowedEgressTarget, AppInstallation, AppInstallationId, ChannelId, ChannelMember,
    ChannelMemberRole, EgressTargetId, MemberFreeze, MemberId, NewAuditEvent, NewEgressTarget,
    NewSecret, Secret, ThreadId, ThreadReviewRequirement, WorkspaceId,
};
use sqlx::SqlitePool;

use super::{
    apps, audit, channel_members, egress_targets, land_gate, member_freezes, member_skills,
    reviews, secrets,
};
use crate::{error::StoreError, AuditFor};

/// Run `change`'s result through `audit` only when `recorded(result)` holds,
/// in the same transaction.
macro_rules! audited {
    ($pool:expr, |$tx:ident| $change:expr, $result:ident => $event:expr) => {{
        let mut $tx = $pool.begin().await?;
        let $result = $change;
        if let Some(event) = $event {
            audit::append_counted(&mut $tx, event).await?;
        }
        $tx.commit().await?;
        Ok($result)
    }};
}

pub async fn freeze_member(
    pool: &SqlitePool,
    member_id: MemberId,
    frozen_by: MemberId,
    reason: Option<&str>,
    audit_for: AuditFor<(MemberFreeze, u64)>,
) -> Result<(MemberFreeze, u64), StoreError> {
    audited!(pool, |tx| member_freezes::freeze_on(&mut tx, member_id, frozen_by, reason).await?,
        result => Some(audit_for(&result)))
}

pub async fn unfreeze_member(
    pool: &SqlitePool,
    member_id: MemberId,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    audited!(pool, |tx| member_freezes::unfreeze_on(&mut tx, member_id).await?,
        unfrozen => unfrozen.then_some(event))
}

pub async fn add_channel_member(
    pool: &SqlitePool,
    channel_id: ChannelId,
    member_id: MemberId,
    role: ChannelMemberRole,
    audit_for: AuditFor<ChannelMember>,
) -> Result<ChannelMember, StoreError> {
    audited!(pool, |tx| channel_members::add_on(&mut tx, channel_id, member_id, role).await?,
        member => Some(audit_for(&member)))
}

pub async fn remove_channel_member(
    pool: &SqlitePool,
    channel_id: ChannelId,
    member_id: MemberId,
    event: NewAuditEvent,
) -> Result<(), StoreError> {
    audited!(pool, |tx| channel_members::remove_on(&mut tx, channel_id, member_id).await?,
        _removed => Some(event))
}

/// Set a thread's review requirement, reading the one it replaces in the same
/// transaction. A caller that may not lower it passes `allow_lower = false`,
/// and a write that would lower it is refused here — a check made before the
/// transaction can be outrun by a concurrent change.
pub async fn set_review_requirement(
    pool: &SqlitePool,
    thread_id: ThreadId,
    required_count: i64,
    allow_lower: bool,
    audit_for: AuditFor<(i64, ThreadReviewRequirement)>,
) -> Result<(i64, ThreadReviewRequirement), StoreError> {
    let mut tx = pool.begin().await?;
    let previous = reviews::get_requirement_on(&mut tx, thread_id)
        .await?
        .map(|r| r.required_count)
        .unwrap_or(0);
    if required_count < previous && !allow_lower {
        return Err(StoreError::Conflict(crate::REVIEW_LOWER_REFUSAL.into()));
    }
    let requirement = reviews::set_requirement_on(&mut tx, thread_id, required_count).await?;
    let change = (previous, requirement);
    audit::append_counted(&mut tx, audit_for(&change)).await?;
    tx.commit().await?;
    Ok(change)
}

pub async fn clear_review_requirement(
    pool: &SqlitePool,
    thread_id: ThreadId,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    audited!(pool, |tx| reviews::clear_requirement_on(&mut tx, thread_id).await?,
        cleared => cleared.then_some(event))
}

pub async fn remove_reviewer(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    audited!(pool, |tx| reviews::remove_reviewer_on(&mut tx, thread_id, member_id).await?,
        removed => removed.then_some(event))
}

pub async fn clear_land_gate(
    pool: &SqlitePool,
    thread_id: ThreadId,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    audited!(pool, |tx| land_gate::clear_on(&mut tx, thread_id).await?,
        cleared => cleared.then_some(event))
}

/// Grant a skill that a gate reads as approval authority.
pub async fn grant_governance_skill(
    pool: &SqlitePool,
    member_id: MemberId,
    skill: &str,
    event: NewAuditEvent,
) -> Result<(), StoreError> {
    audited!(pool, |tx| member_skills::add_on(&mut tx, member_id, skill).await?,
        _granted => Some(event))
}

pub async fn allow_egress_target(
    pool: &SqlitePool,
    new: NewEgressTarget,
    audit_for: AuditFor<AllowedEgressTarget>,
) -> Result<AllowedEgressTarget, StoreError> {
    audited!(pool, |tx| egress_targets::allow_on(&mut tx, new).await?,
        target => Some(audit_for(&target)))
}

pub async fn revoke_egress_target(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    id: EgressTargetId,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    audited!(pool, |tx| egress_targets::revoke_on(&mut tx, workspace_id, id).await?,
        revoked => revoked.then_some(event))
}

pub async fn revoke_app_installation(
    pool: &SqlitePool,
    id: AppInstallationId,
    audit_for: AuditFor<AppInstallation>,
) -> Result<AppInstallation, StoreError> {
    audited!(pool, |tx| apps::revoke_installation_on(&mut tx, id).await?,
        installation => Some(audit_for(&installation)))
}

/// Store a workspace credential. The record names it; the value never enters
/// the audit row.
pub async fn create_secret(
    pool: &SqlitePool,
    new: NewSecret,
    audit_for: AuditFor<Secret>,
) -> Result<Secret, StoreError> {
    audited!(pool, |tx| secrets::create_on(&mut tx, new).await?,
        secret => Some(audit_for(&secret)))
}

pub async fn delete_secret(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    name: &str,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    audited!(pool, |tx| secrets::delete_on(&mut tx, workspace_id, name).await?,
        deleted => deleted.then_some(event))
}
