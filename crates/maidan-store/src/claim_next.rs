//! The thread `claim_next` hands a member, as one query that both backends and
//! both scopes (one channel, or a whole workspace) build.
//!
//! The rules live here once so the channel route and the workspace route cannot
//! drift: a filter added to one and forgotten in the other would hand out work
//! the other refuses. The workspace scope is the channel scope without its
//! channel predicate, and nothing else.
//!
//! A thread is claimable when it is open and live, unheld or its lease lapsed,
//! every dependency is finished, the claimer holds every skill it requires, it
//! has no pending approval gate, it is neither parked unclaimable nor blocked,
//! the claimer is not frozen, and the thread is not already over any budget
//! (tokens, usd, turns, or wall, counting the time a lapsed claim worked).
//! It goes only to a member who may read it:
//! the channel is in the member's workspace, a `__dm__` thread needs the member
//! in its DM or group DM, a private channel needs a `channel_members` row. The
//! read rule is `maidan_auth::authorize_thread`'s, in SQL
//! ([`crate::thread_access`]), and must agree with it. The channel route used to lean on the route's channel check alone, which
//! exempts the shared `__dm__` channel, so any member of a workspace could be
//! handed a DM thread between two others.

use maidan_types::{ChannelId, WorkspaceId};

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
    } = sql;
    let in_scope = match scope {
        ClaimScope::Channel(_) => format!("cand.channel_id = {scope_id}"),
        ClaimScope::Workspace(_) => format!("ch.workspace_id = {scope_id}"),
    };
    let readable = readable_by(member, dm_channel);
    format!(
        "SELECT {columns} FROM maidan_threads cand
         JOIN maidan_channels ch ON ch.id = cand.channel_id
         LEFT JOIN maidan_thread_priorities p ON p.thread_id = cand.id
         WHERE {in_scope} AND cand.tombstoned_at IS NULL AND cand.state = 'open'
           AND (cand.assignee_id IS NULL
                OR (cand.assignment_expires_at IS NOT NULL AND cand.assignment_expires_at < {now}))
           AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_dependencies d
               JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
               WHERE d.thread_id = cand.id AND dep.state NOT IN ('closed', 'archived')
           )
           AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_required_skills trs
               WHERE trs.thread_id = cand.id
                 AND NOT EXISTS (
                     SELECT 1 FROM maidan_member_skills ms
                     WHERE ms.member_id = {member} AND ms.skill = trs.skill
                 )
           )
           AND NOT EXISTS (
               SELECT 1 FROM maidan_approval_gates g
               WHERE g.thread_id = cand.id AND g.state = 'pending'
           )
           AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_unclaimable u WHERE u.thread_id = cand.id
           )
           AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = cand.id
           )
           AND NOT EXISTS (
               SELECT 1 FROM maidan_member_freezes f WHERE f.member_id = {member}
           )
           AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_budgets b
               WHERE b.thread_id = cand.id
                 AND (
                   (b.max_tokens > 0 AND b.used_tokens >= b.max_tokens)
                   OR (b.max_usd_micros > 0 AND b.used_usd_micros >= b.max_usd_micros)
                   OR (b.max_turns > 0 AND b.used_turns >= b.max_turns)
                   OR (b.max_wall_secs > 0
                       AND b.used_wall_secs + ({lapsed_worked_secs}) >= b.max_wall_secs)
                 )
           )
           AND {readable}
         ORDER BY (COALESCE(p.priority, 0) + {hours_waiting}) DESC, cand.created_at ASC, cand.id ASC
         LIMIT 1"
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
