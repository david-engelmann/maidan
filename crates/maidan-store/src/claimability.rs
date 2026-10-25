//! Whether an open thread `cand` may be handed out now, as SQL. Each rule is
//! defined here once, and two places use it: [`crate::claim_next`] ANDs all of
//! them to pick a thread, and [`crate::queue_counts`] partitions the queue by
//! them. So `ready` counts exactly the threads `claim_next` would hand to a
//! claimer with no skill, repository, engine or freeze restriction. Before,
//! each place spelled out its own copy, and the counts drifted: a gated,
//! over-budget or already-finished thread was counted `ready`, but
//! `claim_next` refused it.
//!
//! Rules about the claimer (skills, freezes, repository, engine, read access)
//! stay in [`crate::claim_next`]. They depend on who is asking, not on the
//! thread.

/// The parts each backend spells its own way.
pub(crate) struct ThreadSql<'a> {
    /// The current time, compared with a lease deadline.
    pub now: &'a str,
    /// Seconds a lapsed claim on `cand` worked, from acknowledgement to
    /// deadline, or 0.
    pub lapsed_worked_secs: &'a str,
    /// True when transition `tr` moved `cand` to `open` at or after its
    /// result `r` was produced.
    pub requeued_after_result: &'a str,
}

/// Held on a live lease, or held with no lease at all.
pub(crate) fn held(now: &str) -> String {
    format!(
        "(cand.assignee_id IS NOT NULL
          AND (cand.assignment_expires_at IS NULL OR cand.assignment_expires_at >= {now}))"
    )
}

/// Unheld, or held on a lease that lapsed: what `claim_next` may take over.
pub(crate) fn available(now: &str) -> String {
    format!(
        "(cand.assignee_id IS NULL
          OR (cand.assignment_expires_at IS NOT NULL AND cand.assignment_expires_at < {now}))"
    )
}

/// Parked unclaimable by an operator.
pub(crate) const PARKED: &str =
    "EXISTS (SELECT 1 FROM maidan_thread_unclaimable u WHERE u.thread_id = cand.id)";

/// A live duplicate (Context Economics C8b): created with a fingerprint
/// identical to a task that is still open or in review, so the work goes to
/// that original instead. Clearing the link makes it claimable again.
pub(crate) const DUPLICATE: &str = "EXISTS (SELECT 1 FROM maidan_task_fingerprints fp
                        JOIN maidan_threads orig ON orig.id = fp.duplicate_of
                        WHERE fp.thread_id = cand.id AND orig.tombstoned_at IS NULL
                          AND orig.state IN ('open', 'in_review'))";

/// A likely duplicate by Linear issue (C8 follow-up): a change thread whose
/// `linear_issue` already has another live change thread in review in the
/// same workspace, whatever engine either is routed to. Titles often differ
/// ("Session: ENG-1 ..." against "ENG-1 ..."), so the fingerprint alone misses
/// it. Closing the thread, or clearing its duplicate link (which records
/// `cleared_at`), lifts the hold; so does the other thread leaving review.
pub(crate) const ISSUE_IN_REVIEW: &str = "EXISTS (SELECT 1 FROM maidan_thread_attributes ia
                        JOIN maidan_channels ich ON ich.id = cand.channel_id
                        JOIN maidan_thread_attributes oa
                          ON oa.linear_issue = ia.linear_issue AND oa.thread_id <> ia.thread_id
                        JOIN maidan_threads o ON o.id = oa.thread_id
                        JOIN maidan_channels och ON och.id = o.channel_id
                        WHERE ia.thread_id = cand.id AND ia.kind = 'change'
                          AND ia.linear_issue IS NOT NULL
                          AND oa.kind = 'change'
                          AND och.workspace_id = ich.workspace_id
                          AND o.tombstoned_at IS NULL AND o.state = 'in_review'
                          AND NOT EXISTS (SELECT 1 FROM maidan_task_fingerprints cf
                                          WHERE cf.thread_id = cand.id
                                            AND cf.cleared_at IS NOT NULL))";

/// Carries an explicit block.
pub(crate) const BLOCKED: &str =
    "EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = cand.id)";

/// Depends on a thread that is not yet closed or archived.
pub(crate) const WAITING: &str = "EXISTS (SELECT 1 FROM maidan_thread_dependencies d
                        JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                        WHERE d.thread_id = cand.id AND dep.state NOT IN ('closed', 'archived'))";

/// Has an approval gate still pending.
pub(crate) const GATED: &str = "EXISTS (SELECT 1 FROM maidan_approval_gates g
                        WHERE g.thread_id = cand.id AND g.state = 'pending')";

/// Already at or over a budget limit (tokens, usd, turns, or wall time,
/// counting the time a lapsed claim worked).
pub(crate) fn over_budget(sql: &ThreadSql<'_>) -> String {
    let lapsed = sql.lapsed_worked_secs;
    format!(
        "EXISTS (SELECT 1 FROM maidan_thread_budgets bu
                 WHERE bu.thread_id = cand.id
                   AND ((bu.max_tokens > 0 AND bu.used_tokens >= bu.max_tokens)
                     OR (bu.max_usd_micros > 0 AND bu.used_usd_micros >= bu.max_usd_micros)
                     OR (bu.max_turns > 0 AND bu.used_turns >= bu.max_turns)
                     OR (bu.max_wall_secs > 0
                         AND bu.used_wall_secs + ({lapsed}) >= bu.max_wall_secs)))"
    )
}

/// Has a stored result and was not moved back to `open` after it (#1499).
pub(crate) fn finished(sql: &ThreadSql<'_>) -> String {
    let requeued = sql.requeued_after_result;
    format!(
        "EXISTS (SELECT 1 FROM maidan_thread_results r
                 WHERE r.thread_id = cand.id
                   AND NOT EXISTS (
                       SELECT 1 FROM maidan_thread_transitions tr
                       WHERE tr.thread_id = cand.id AND tr.to_state = 'open'
                         AND {requeued}))"
    )
}

/// Its footprint overlaps a live claim or names a paused repository.
pub(crate) fn deferred(now: &str) -> String {
    crate::footprints::deferred("cand.id", now)
}

/// Kept off the queue until someone acts (parked, out of budget, already
/// finished, a live duplicate, or a likely duplicate of an issue in review):
/// queue depth's `unclaimable`.
pub(crate) fn withheld(sql: &ThreadSql<'_>) -> String {
    format!(
        "({PARKED} OR {DUPLICATE} OR {ISSUE_IN_REVIEW} OR {} OR {})",
        over_budget(sql),
        finished(sql)
    )
}

/// Waiting on something that clears by itself or by a decision (a block, a
/// dependency, an approval gate, a footprint conflict): queue depth's
/// `blocked`.
pub(crate) fn stalled(now: &str) -> String {
    format!("({BLOCKED} OR {WAITING} OR {GATED} OR {})", deferred(now))
}

/// Open, live, and claimable now by a claimer with no restrictions. This is
/// the thread half of `claim_next`'s predicate.
pub(crate) fn claimable(sql: &ThreadSql<'_>) -> String {
    format!(
        "(cand.tombstoned_at IS NULL AND cand.state = 'open' AND {}
          AND NOT {} AND NOT {})",
        available(sql.now),
        withheld(sql),
        stalled(sql.now)
    )
}
