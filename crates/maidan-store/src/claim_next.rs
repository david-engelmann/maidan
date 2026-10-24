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
    /// Placeholder bound to the service lane the claimer runs in, or NULL
    /// for none. Lane is a tiebreak, never a filter: among threads of equal
    /// aged priority, a lane claimer is handed threads in its lane first, then
    /// unlaned ones, then other lanes; a claimer with no lane is handed
    /// unlaned threads first. Priority and wait still rank first, so an
    /// urgent thread is never held back by lane.
    pub lane: &'a str,
    /// True when transition `tr` moved `cand` to `open` at or after its
    /// result `r` was produced. Each backend compares the two times its own
    /// way, since SQLite stores them as text.
    pub requeued_after_result: &'a str,
    /// Placeholder bound to the app's `now` less [`WARM_HOLD_SECS`].
    pub warm_hold_since: &'a str,
    /// Placeholder bound to the app's `now` less [`WARM_TTL_SECS`].
    pub warm_ttl_since: &'a str,
    /// True when the time in column `{col}` is after the time `{t}`. Each
    /// backend compares times its own way, since SQLite stores them as text
    /// in more than one format. Every age is measured against a cutoff the
    /// app computed from its own clock, never the database clock.
    pub later_than: &'a str,
}

/// `SELECT {columns}` of the one thread `claim_next` would give the claimer in
/// `scope`: the highest priority aged by its wait, then one [`warm_for`] the
/// claimer, then the oldest. The
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
        lane,
        requeued_after_result,
        warm_hold_since,
        warm_ttl_since,
        later_than,
    } = sql;
    let in_scope = match scope {
        ClaimScope::Channel(_) => format!("cand.channel_id = {scope_id}"),
        ClaimScope::Workspace(_) => format!("ch.workspace_id = {scope_id}"),
    };
    let readable = readable_by(member, dm_channel);
    let warm = warm_for(member);
    let warming = sibling_warming(now, warm_hold_since, warm_ttl_since, later_than);
    let refused = crate::review_family::refused_to(member);
    let claimable = claimability::claimable(&ThreadSql {
        now,
        lapsed_worked_secs,
        requeued_after_result,
    });
    let lane_rank = lane_rank(lane);
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
         ORDER BY (COALESCE(p.priority, 0) + {hours_waiting}) DESC,
                  {warm} DESC, {lane_rank} ASC, cand.created_at ASC, cand.id ASC
         LIMIT 1"
    )
}

/// 1 when `member` was the last to hold `cand`, else 0 (Context Economics
/// C6). The agent that last held a thread most likely still has its prompt
/// prefix cached, so a re-queued thread (changes requested, or a lapsed
/// claim) goes back to it ahead of older work of the same aged priority.
/// Only a tiebreak: it never outranks priority or waiting, and never
/// withholds a thread from anyone else.
pub(crate) fn warm_for(member: &str) -> String {
    format!(
        "(CASE WHEN EXISTS (
             SELECT 1 FROM maidan_thread_workers w
             WHERE w.thread_id = cand.id AND w.member_id = {member}
               AND w.last_held_seq = (
                   SELECT MAX(w2.last_held_seq) FROM maidan_thread_workers w2
                   WHERE w2.thread_id = cand.id)
         ) THEN 1 ELSE 0 END)"
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
/// shares (Context Economics C5: warm, then fan out). Only siblings that truly
/// share a prefix count (see [`shares_prefix`]), so plan, review and manual
/// sub-threads of one parent are never held back. One sibling is handed out
/// first; the rest wait while it is held on a live claim taken in the last
/// [`WARM_HOLD_SECS`] that has reported no usage yet, unless some sibling
/// reported usage within [`WARM_TTL_SECS`]. The first usage report means the
/// provider has written the shared prefix, so the rest read it instead of
/// each writing it cold.
pub(crate) fn sibling_warming(
    now: &str,
    hold_since: &str,
    ttl_since: &str,
    later_than: &str,
) -> String {
    let claimed_recently = later_than
        .replace("{col}", "sib.claimed_at")
        .replace("{t}", hold_since);
    let used_recently = later_than
        .replace("{col}", "u.accepted_at")
        .replace("{t}", ttl_since);
    let held_shares = shares_prefix("sib");
    let used_shares = shares_prefix("usib");
    format!(
        "(cand.parent_thread_id IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM maidan_threads sib
              WHERE sib.parent_thread_id = cand.parent_thread_id
                AND sib.id <> cand.id
                AND {held_shares}
                AND sib.assignee_id IS NOT NULL
                AND sib.claim_lease_id IS NOT NULL
                AND (sib.assignment_expires_at IS NULL OR sib.assignment_expires_at >= {now})
                AND sib.claimed_at IS NOT NULL
                AND {claimed_recently}
                AND NOT EXISTS (
                    SELECT 1 FROM maidan_usage_ledger u
                    WHERE u.thread_id = sib.id AND u.claim_lease_id = sib.claim_lease_id
                )
          )
          AND NOT EXISTS (
              SELECT 1 FROM maidan_usage_ledger u
              JOIN maidan_threads usib ON usib.id = u.thread_id
              WHERE usib.parent_thread_id = cand.parent_thread_id
                AND {used_shares}
                AND {used_recently}
          ))"
    )
}

/// True when sibling `{sib}` shares `cand`'s prompt prefix: both are children
/// of one recipe run (built from one spec, so one shared preamble), or the two
/// have the same task fingerprint (an identical brief and inputs).
fn shares_prefix(sib: &str) -> String {
    format!(
        "(EXISTS (
              SELECT 1 FROM maidan_recipe_runs rr
              WHERE rr.root_thread_id = cand.parent_thread_id
          )
          OR EXISTS (
              SELECT 1 FROM maidan_task_fingerprints fc
              JOIN maidan_task_fingerprints fs
                ON fs.workspace_id = fc.workspace_id AND fs.fingerprint = fc.fingerprint
              WHERE fc.thread_id = cand.id AND fs.thread_id = {sib}.id
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

/// 0 for a thread in the claimer's `lane` (an unlaned thread when the claimer
/// has none), 1 for an unlaned thread under a lane claimer, 2 for any other
/// lane: the tiebreak after aged priority, so lane routes without reserving
/// and without overriding priority.
fn lane_rank(lane: &str) -> String {
    let cand_lane =
        "(SELECT la.lane FROM maidan_thread_attributes la WHERE la.thread_id = cand.id)";
    format!(
        "CASE WHEN COALESCE({cand_lane}, '') = COALESCE({lane}, '') THEN 0 \
         WHEN {cand_lane} IS NULL THEN 1 ELSE 2 END"
    )
}
