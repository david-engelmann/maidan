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
           AND mine.match_namespace <> 'slot'
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

/// How many other threads hold slot `key` in workspace `ws` under a live
/// claim, `thread` excluded. Slots count across repositories and modes.
fn slot_in_use(ws: &str, key: &str, thread: &str, now: &str) -> String {
    let live = live_claim("sheld", now);
    format!(
        "(SELECT COUNT(DISTINCT sf.thread_id) FROM maidan_thread_footprints sf
          JOIN maidan_threads sheld ON sheld.id = sf.thread_id
          WHERE sf.workspace_id = {ws} AND sf.match_namespace = 'slot'
            AND sf.match_key = {key} AND sf.thread_id <> {thread}
            AND {live})"
    )
}

/// The joins and predicates that pick the slot items of `thread` (`slots`)
/// whose capacity (`cap`) other live claims already fill.
fn full_slots(thread: &str, now: &str) -> String {
    let in_use = slot_in_use(
        "slots.workspace_id",
        "slots.match_key",
        "slots.thread_id",
        now,
    );
    format!(
        "FROM maidan_thread_footprints slots
         JOIN maidan_slot_capacities cap
              ON cap.workspace_id = slots.workspace_id AND cap.slot = slots.match_key
         WHERE slots.thread_id = {thread} AND slots.match_namespace = 'slot'
           AND cap.capacity <= {in_use}"
    )
}

/// True when one of `thread`'s slots is at capacity.
pub(crate) fn slot_full(thread: &str, now: &str) -> String {
    format!("EXISTS (SELECT 1 {})", full_slots(thread, now))
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
/// overlaps a live claim, it names a paused repository, or one of its slots
/// is at capacity.
pub(crate) fn deferred(thread: &str, now: &str) -> String {
    format!(
        "({} OR {} OR {})",
        conflicts_with_live_claims(thread, now),
        repo_paused(thread),
        slot_full(thread, now)
    )
}

/// The joins and predicates that pair each full slot of `thread` (`slots`)
/// with the live claims holding it (`theirs`, on thread `held`).
fn full_slot_holders(thread: &str, now: &str) -> String {
    let live = live_claim("held", now);
    let in_use = slot_in_use(
        "slots.workspace_id",
        "slots.match_key",
        "slots.thread_id",
        now,
    );
    format!(
        "FROM maidan_thread_footprints slots
         JOIN maidan_slot_capacities cap
              ON cap.workspace_id = slots.workspace_id AND cap.slot = slots.match_key
         JOIN maidan_thread_footprints theirs
              ON theirs.workspace_id = slots.workspace_id
             AND theirs.match_namespace = 'slot'
             AND theirs.match_key = slots.match_key
             AND theirs.thread_id <> slots.thread_id
         JOIN maidan_threads held ON held.id = theirs.thread_id
         WHERE slots.thread_id = {thread} AND slots.match_namespace = 'slot'
           AND {live}
           AND cap.capacity <= {in_use}"
    )
}

/// `SELECT` of the live-claim items that keep `thread` from its footprint:
/// the items it overlaps, and the holders of each of its slots at capacity,
/// as [`conflict_from_row`] reads them.
pub(crate) fn conflicts_select(thread: &str, now: &str) -> String {
    let columns = "theirs.thread_id, held.assignee_id, held.assignment_expires_at,
                theirs.repo, theirs.kind, theirs.value, theirs.mode";
    format!(
        "SELECT {columns} {}
         UNION
         SELECT {columns} {}
         ORDER BY 1, 5, 6",
        overlapping(thread, now),
        full_slot_holders(thread, now)
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
