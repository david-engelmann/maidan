//! The thread `claim_next` hands a member, as one query that both backends and
//! both scopes (one channel, or a whole workspace) build.
//!
//! The rules live here once so the channel route and the workspace route cannot
//! drift: a filter added to one and forgotten in the other would hand out work
//! the other refuses. The workspace scope is the channel scope without its
//! channel predicate, and nothing else.
//!
//! The thread's own rules are [`crate::claimability`]'s, shared with the
//! queue counts. A thread is claimable when it is open and live, unheld or its lease lapsed,
//! every dependency is finished, the claimer holds every skill it requires, it
//! has no pending approval gate, it is neither parked unclaimable nor blocked,
//! the claimer is not frozen, the thread is not already over any budget
//! (tokens, usd, turns, or wall, counting the time a lapsed claim worked), and
//! its footprint overlaps no live claim and names no paused repository
//! ([`crate::footprints`]): a swarm claiming from one queue is never handed
//! two pieces of the same work. A claimer that serves one repository (a
//! worker whose checkout is that repository) can ask for work whose footprint
//! names it; a thread without a footprint is then never handed to it. A
//! claimer that serves one engine can ask only for work routed to that
//! engine; work routed to another engine, or not routed at all, is never
//! handed to it.
//! A thread that already has a stored result is finished work, so it is not
//! handed out again (#1499) unless it was moved back to `open` after that
//! result was produced: a reopen or a reviewer's request for changes is the
//! explicit re-queue. Without this, a finished task nobody closed is re-handed
//! the moment its claim lapses.
//! It goes only to a member who may read it:
//! the channel is in the member's workspace, a `__dm__` thread needs the member
//! in its DM or group DM, a private channel needs a `channel_members` row. The
//! read rule is `maidan_auth::authorize_thread`'s, in SQL
//! ([`crate::thread_access`]), and must agree with it. The channel route used to lean on the route's channel check alone, which
//! exempts the shared `__dm__` channel, so any member of a workspace could be
//! handed a DM thread between two others.

use maidan_types::{ChannelId, WorkspaceId};

use crate::claimability::{self, ThreadSql};
use crate::thread_access::readable_thread;

/// Where `claim_next` looks for work.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ClaimScope {
    Channel(ChannelId),
    Workspace(WorkspaceId),
}

impl ClaimScope {
    /// The id the scope's placeholder is bound to.
    pub(crate) fn id(self) -> uuid::Uuid {
        match self {
            Self::Channel(id) => id.0,
            Self::Workspace(id) => id.0,
        }
    }
}

/// The parts of the query each backend spells its own way.
pub(crate) struct ClaimSql<'a> {
    /// Placeholder bound to the claimer's member id.
    pub member: &'a str,
    /// Placeholder bound to [`ClaimScope::id`].
    pub scope: &'a str,
    /// Placeholder bound to [`maidan_types::DM_CHANNEL_NAME`].
    pub dm_channel: &'a str,
    /// The current time, compared with a lease deadline.
    pub now: &'a str,
    /// Whole hours the candidate `cand` has waited since it was created.
    pub hours_waiting: &'a str,
    /// Seconds a lapsed claim on `cand` worked, acknowledgement to deadline,
    /// or 0 when that claim was never acknowledged or the thread is free.
    pub lapsed_worked_secs: &'a str,
    /// Placeholder bound to the repository (`owner/name`, normalized) the
    /// claimer serves, or NULL for any work.
    pub repo: &'a str,
    /// Placeholder bound to the engine the claimer serves, or NULL for any
    /// work. A named engine takes only threads routed to it: a thread with
    /// no engine, or another engine, is not handed over.
    pub engine: &'a str,
    /// True when transition `tr` moved `cand` to `open` at or after its
    /// result `r` was produced. Each backend compares the two times its own
    /// way, since SQLite stores them as text.
    pub requeued_after_result: &'a str,
    /// Whole seconds from the column `{col}` to now, with `{col}` replaced by
    /// the column. Each backend does time arithmetic its own way.
    pub secs_since: &'a str,
}

/// `SELECT {columns}` of the one thread `claim_next` would give the claimer in
/// `scope`: the highest priority aged by its wait, then the oldest. The
/// candidate thread is aliased `cand` and its channel `ch`; the caller appends
/// any row lock.
pub(crate) fn candidate_select(scope: ClaimScope, columns: &str, sql: &ClaimSql<'_>) -> String {
    let ClaimSql {
        member,
        scope: scope_id,
        dm_channel,
        now,
        hours_waiting,
        lapsed_worked_secs,
        repo,
        engine,
        requeued_after_result,
        secs_since,
    } = sql;
    let in_scope = match scope {
        ClaimScope::Channel(_) => format!("cand.channel_id = {scope_id}"),
        ClaimScope::Workspace(_) => format!("ch.workspace_id = {scope_id}"),
    };
    let readable = readable_by(member, dm_channel);
    let warming = sibling_warming(now, secs_since);
    let refused = crate::review_family::refused_to(member);
    let claimable = claimability::claimable(&ThreadSql {
        now,
        lapsed_worked_secs,
        requeued_after_result,
    });
    format!(
        "SELECT {columns} FROM maidan_threads cand
         JOIN maidan_channels ch ON ch.id = cand.channel_id
         LEFT JOIN maidan_thread_priorities p ON p.thread_id = cand.id
         WHERE {in_scope} AND {claimable}
           AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_required_skills trs
               WHERE trs.thread_id = cand.id
                 AND NOT EXISTS (
                     SELECT 1 FROM maidan_member_skills ms
                     WHERE ms.member_id = {member} AND ms.skill = trs.skill
                 )
           )
           AND NOT EXISTS (
               SELECT 1 FROM maidan_member_freezes f WHERE f.member_id = {member}
           )
           AND ({repo} IS NULL OR EXISTS (
               SELECT 1 FROM maidan_thread_footprints rf
               WHERE rf.thread_id = cand.id AND rf.repo = {repo}
           ))
           AND ({engine} IS NULL OR EXISTS (
               SELECT 1 FROM maidan_thread_attributes a
               WHERE a.thread_id = cand.id AND a.engine = {engine}
           ))
           AND NOT {refused}
           AND {readable}
           AND NOT {warming}
         ORDER BY (COALESCE(p.priority, 0) + {hours_waiting}) DESC, cand.created_at ASC, cand.id ASC
         LIMIT 1"
    )
}

/// How long a sibling's first claim holds the others back, at most, waiting
/// for its first usage report. Past this the siblings fan out anyway, so a
/// stalled or silent first claimer never starves the group.
pub(crate) const WARM_HOLD_SECS: i64 = 120;

/// How long a sibling's usage report keeps the group's prefix warm: the
/// 5-minute cache TTL. Within it, siblings fan out without waiting again.
pub(crate) const WARM_TTL_SECS: i64 = 300;

/// True when `cand` should wait for a sibling to warm the prefix the group
/// shares (Context Economics C5: warm, then fan out). Siblings are the
/// children of one parent thread, such as a recipe's children. One sibling is
/// handed out first; the rest wait while it is held on a live claim taken in
/// the last [`WARM_HOLD_SECS`] that has reported no usage yet, unless some
/// sibling reported usage within [`WARM_TTL_SECS`]. The first usage report
/// means the provider has written the shared prefix, so the rest read it
/// instead of each writing it cold.
pub(crate) fn sibling_warming(now: &str, secs_since: &str) -> String {
    let claimed_age = secs_since.replace("{col}", "sib.claimed_at");
    let usage_age = secs_since.replace("{col}", "u.accepted_at");
    format!(
        "(cand.parent_thread_id IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM maidan_threads sib
              WHERE sib.parent_thread_id = cand.parent_thread_id
                AND sib.id <> cand.id
                AND sib.assignee_id IS NOT NULL
                AND sib.claim_lease_id IS NOT NULL
                AND (sib.assignment_expires_at IS NULL OR sib.assignment_expires_at >= {now})
                AND sib.claimed_at IS NOT NULL
                AND {claimed_age} < {WARM_HOLD_SECS}
                AND NOT EXISTS (
                    SELECT 1 FROM maidan_usage_ledger u
                    WHERE u.thread_id = sib.id AND u.claim_lease_id = sib.claim_lease_id
                )
          )
          AND NOT EXISTS (
              SELECT 1 FROM maidan_usage_ledger u
              JOIN maidan_threads sib ON sib.id = u.thread_id
              WHERE sib.parent_thread_id = cand.parent_thread_id
                AND {usage_age} < {WARM_TTL_SECS}
          ))"
    )
}

/// True when `member` may read `cand`: [`readable_thread`], the one SQL form
/// of the thread read rule, with the workspace read from the member's own row
/// rather than trusted from the caller, so a scope naming another tenant finds
/// nothing. The queue counts filter with it too, so a count and a claim agree
/// on whose work a thread is.
pub(crate) fn readable_by(member: &str, dm_channel: &str) -> String {
    let own_workspace = format!(
        "(SELECT claimer.workspace_id FROM maidan_members claimer WHERE claimer.id = {member})"
    );
    readable_thread("cand.id", &own_workspace, member, dm_channel)
}
