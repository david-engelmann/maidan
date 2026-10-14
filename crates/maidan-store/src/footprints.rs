//! Claim footprints, the backend-neutral half: the SQL both backends build
//! for "does this thread's footprint overlap a live claim", and the row
//! grouping for reads. The rules are in [`maidan_types::footprint`].

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use maidan_types::{
    ActiveClaim, FootprintConflict, FootprintItem, FootprintKind, FootprintMode, MemberId, ThreadId,
};

use crate::error::StoreError;

/// True when thread `held` (an alias of `maidan_threads`) holds a live claim
/// as of `now`: an assignee, the thread open and live, and a lease that has
/// not lapsed (a claim taken without a lease holds until released).
pub(crate) fn live_claim(held: &str, now: &str) -> String {
    format!(
        "({held}.assignee_id IS NOT NULL AND {held}.state = 'open' AND {held}.tombstoned_at IS NULL
          AND ({held}.assignment_expires_at IS NULL OR {held}.assignment_expires_at >= {now}))"
    )
}

/// The joins and predicates that pair the items of `thread` (`mine`) with the
/// items of other threads' live claims they overlap (`theirs`, on thread
/// `held`).
fn overlapping(thread: &str, now: &str) -> String {
    let live = live_claim("held", now);
    format!(
        "FROM maidan_thread_footprints mine
         JOIN maidan_thread_footprints theirs
              ON theirs.workspace_id = mine.workspace_id
             AND theirs.repo = mine.repo
             AND theirs.match_namespace = mine.match_namespace
             AND theirs.thread_id <> mine.thread_id
         JOIN maidan_threads held ON held.id = theirs.thread_id
         WHERE mine.thread_id = {thread}
           AND {live}
           AND (mine.mode = 'exclusive' OR theirs.mode = 'exclusive'
                OR mine.match_namespace = 'branch')
           AND CASE WHEN mine.match_namespace = 'path'
                    THEN substr(mine.match_key, 1, length(theirs.match_key)) = theirs.match_key
                      OR substr(theirs.match_key, 1, length(mine.match_key)) = mine.match_key
                    ELSE mine.match_key = theirs.match_key
               END"
    )
}

/// True when `thread`'s footprint overlaps another thread's live claim, so
/// claiming it would put two agents on the same work.
pub(crate) fn conflicts_with_live_claims(thread: &str, now: &str) -> String {
    format!("EXISTS (SELECT 1 {})", overlapping(thread, now))
}

/// True when `thread`'s footprint names a repository paused in its
/// workspace ([`maidan_types::RepoPause`]).
pub(crate) fn repo_paused(thread: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM maidan_thread_footprints pf
                 JOIN maidan_repo_pauses rp
                      ON rp.workspace_id = pf.workspace_id AND rp.repo = pf.repo
                 WHERE pf.thread_id = {thread})"
    )
}

/// True when the claim queue holds `thread` back for its footprint: it
/// overlaps a live claim, or it names a paused repository.
pub(crate) fn deferred(thread: &str, now: &str) -> String {
    format!(
        "({} OR {})",
        conflicts_with_live_claims(thread, now),
        repo_paused(thread)
    )
}

/// `SELECT` of the live-claim items `thread`'s footprint overlaps, as
/// [`conflict_from_columns`] reads them.
pub(crate) fn conflicts_select(thread: &str, now: &str) -> String {
    format!(
        "SELECT DISTINCT theirs.thread_id, held.assignee_id, held.assignment_expires_at,
                theirs.repo, theirs.kind, theirs.value, theirs.mode
         {}
         ORDER BY theirs.thread_id, theirs.kind, theirs.value",
        overlapping(thread, now)
    )
}

/// The columns of one stored footprint item, as both backends read them.
pub(crate) struct ItemRow {
    pub(crate) thread_id: uuid::Uuid,
    pub(crate) assignee_id: Option<uuid::Uuid>,
    pub(crate) lease_expires_at: Option<DateTime<Utc>>,
    pub(crate) repo: String,
    pub(crate) mode: String,
    pub(crate) kind: String,
    pub(crate) value: String,
}

fn parse_mode(mode: &str) -> Result<FootprintMode, StoreError> {
    FootprintMode::parse(mode)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown footprint mode `{mode}`")))
}

fn parse_kind(kind: &str) -> Result<FootprintKind, StoreError> {
    FootprintKind::parse(kind)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown footprint kind `{kind}`")))
}

pub(crate) fn conflict_from_row(row: ItemRow) -> Result<FootprintConflict, StoreError> {
    Ok(FootprintConflict {
        thread_id: ThreadId(row.thread_id),
        assignee_id: MemberId(
            row.assignee_id
                .ok_or_else(|| StoreError::InvalidInput("a live claim has no assignee".into()))?,
        ),
        lease_expires_at: row.lease_expires_at,
        repo: row.repo,
        kind: parse_kind(&row.kind)?,
        value: row.value,
        mode: parse_mode(&row.mode)?,
    })
}

/// One footprint per thread from its item rows, in the order the rows came.
pub(crate) fn group_active(rows: Vec<ItemRow>) -> Result<Vec<ActiveClaim>, StoreError> {
    let mut order: Vec<uuid::Uuid> = Vec::new();
    let mut claims: BTreeMap<uuid::Uuid, ActiveClaim> = BTreeMap::new();
    for row in rows {
        let item = FootprintItem {
            kind: parse_kind(&row.kind)?,
            value: row.value,
        };
        if let Some(claim) = claims.get_mut(&row.thread_id) {
            claim.items.push(item);
            continue;
        }
        order.push(row.thread_id);
        claims.insert(
            row.thread_id,
            ActiveClaim {
                thread_id: ThreadId(row.thread_id),
                assignee_id: MemberId(row.assignee_id.ok_or_else(|| {
                    StoreError::InvalidInput("a live claim has no assignee".into())
                })?),
                lease_expires_at: row.lease_expires_at,
                repo: row.repo,
                mode: parse_mode(&row.mode)?,
                items: vec![item],
            },
        );
    }
    Ok(order
        .into_iter()
        .filter_map(|id| claims.remove(&id))
        .collect())
}

/// A thread's declared footprint from its item rows; `None` when it has none.
pub(crate) fn footprint_from_rows(
    rows: Vec<ItemRow>,
) -> Result<Option<maidan_types::Footprint>, StoreError> {
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let repo = first.repo.clone();
    let mode = parse_mode(&first.mode)?;
    let items = rows
        .into_iter()
        .map(|row| {
            Ok(FootprintItem {
                kind: parse_kind(&row.kind)?,
                value: row.value,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok(Some(maidan_types::Footprint { repo, mode, items }))
}
