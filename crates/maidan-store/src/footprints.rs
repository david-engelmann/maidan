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

/// Whether two `path`-namespace keys overlap: one is a prefix of the other,
/// the rule [`overlapping`] applies in SQL. A glob's key is what comes before
/// its first wildcard, so `src/bridge/**` covers `src/bridge/x.rs`.
pub(crate) fn path_keys_overlap(a: &str, b: &str) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// The `paths` that a claim holding the `path`-namespace keys `keys` covers,
/// in the order given, at most `limit`.
pub(crate) fn covered_paths(keys: &[String], paths: &[String], limit: usize) -> Vec<String> {
    paths
        .iter()
        .filter(|path| keys.iter().any(|key| path_keys_overlap(key, path)))
        .take(limit)
        .cloned()
        .collect()
}

/// `SELECT` of the `path`-namespace footprint keys of the live claims in
/// workspace `ws` and repository `repo`, thread `exclude` left out, as
/// `(thread_id, match_key)` ordered by thread. Lane, crate, branch and slot
/// items say nothing about files, so they never match a merged path.
pub(crate) fn live_path_keys_select(ws: &str, repo: &str, exclude: &str, now: &str) -> String {
    let live = live_claim("held", now);
    format!(
        "SELECT f.thread_id, f.match_key
         FROM maidan_thread_footprints f
         JOIN maidan_threads held ON held.id = f.thread_id
         WHERE f.workspace_id = {ws} AND f.repo = {repo}
           AND f.match_namespace = 'path' AND f.thread_id <> {exclude}
           AND {live}
         ORDER BY f.thread_id, f.match_key"
    )
}

/// Group `(thread_id, key)` rows, already ordered by thread, into each
/// thread's keys.
pub(crate) fn keys_by_thread(rows: Vec<(uuid::Uuid, String)>) -> Vec<(uuid::Uuid, Vec<String>)> {
    let mut grouped: Vec<(uuid::Uuid, Vec<String>)> = Vec::new();
    for (thread, key) in rows {
        match grouped.last_mut() {
            Some((last, keys)) if *last == thread => keys.push(key),
            _ => grouped.push((thread, vec![key])),
        }
    }
    grouped
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

/// `SELECT` of the open, unheld threads in workspace `ws` that the claim
/// queue defers for their footprint, oldest first, with one flag per reason;
/// only threads `reader` may read when it is not NULL. At most 200.
pub(crate) fn deferred_select(ws: &str, reader: &str, dm_channel: &str, now: &str) -> String {
    let readable = crate::thread_access::readable_thread("cand.id", ws, reader, dm_channel);
    format!(
        "SELECT cand.id AS thread_id, cand.title,
                {} AS overlap, {} AS repo_paused, {} AS slot_full
         FROM maidan_threads cand
         JOIN maidan_channels ch ON ch.id = cand.channel_id
         WHERE ch.workspace_id = {ws}
           AND cand.state = 'open' AND cand.tombstoned_at IS NULL
           AND (cand.assignee_id IS NULL
                OR (cand.assignment_expires_at IS NOT NULL AND cand.assignment_expires_at < {now}))
           AND {}
           AND ({reader} IS NULL OR {readable})
         ORDER BY cand.created_at, cand.id
         LIMIT 200",
        conflicts_with_live_claims("cand.id", now),
        repo_paused("cand.id"),
        slot_full("cand.id", now),
        deferred("cand.id", now),
    )
}

/// The reasons a [`deferred_select`] row carries.
pub(crate) fn deferral_reasons(
    overlap: bool,
    paused: bool,
    slot_full: bool,
) -> Vec<maidan_types::DeferralReason> {
    use maidan_types::DeferralReason::*;
    [
        (overlap, Overlap),
        (paused, RepoPaused),
        (slot_full, SlotFull),
    ]
    .into_iter()
    .filter_map(|(on, reason)| on.then_some(reason))
    .collect()
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
    /// The thread's task attributes, when the query joined them and the
    /// thread has a row; `None` fields read as the defaults.
    pub(crate) task_kind: Option<String>,
    pub(crate) task_engine: Option<String>,
    pub(crate) threat_model: Option<bool>,
    pub(crate) task_size: Option<String>,
    pub(crate) task_acceptance: Option<String>,
    pub(crate) task_tier: Option<String>,
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
                attributes: maidan_types::TaskAttributes {
                    kind: row
                        .task_kind
                        .as_deref()
                        .and_then(maidan_types::TaskKind::parse)
                        .unwrap_or_default(),
                    engine: row
                        .task_engine
                        .as_deref()
                        .and_then(maidan_types::TaskEngine::parse),
                    threat_model: row.threat_model.unwrap_or(false),
                    size: row
                        .task_size
                        .as_deref()
                        .and_then(maidan_types::TaskSize::parse),
                    acceptance: row.task_acceptance.clone(),
                    linear_issue: None,
                    tier: row
                        .task_tier
                        .as_deref()
                        .and_then(maidan_types::TaskTier::parse),
                    review_family: None,
                },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn a_merged_path_is_covered_by_a_prefix_key_either_way() {
        let paths = strings(&["src/bridge/x.rs", "src/gateway/y.rs", "README.md"]);
        assert_eq!(
            covered_paths(&strings(&["src/bridge/"]), &paths, 20),
            strings(&["src/bridge/x.rs"])
        );
        // A path key naming a directory a merged file sits under, or a file
        // inside a directory the merge touched, both overlap.
        assert_eq!(
            covered_paths(&strings(&["src"]), &paths, 20),
            strings(&["src/bridge/x.rs", "src/gateway/y.rs"])
        );
        assert!(covered_paths(&strings(&["src/gateway/z/"]), &paths, 20).is_empty());
        assert_eq!(covered_paths(&strings(&[""]), &paths, 2).len(), 2, "capped");
    }

    #[test]
    fn keys_group_by_thread_in_order() {
        let a = uuid::Uuid::from_u128(1);
        let b = uuid::Uuid::from_u128(2);
        let rows = vec![(a, "x".into()), (a, "y".into()), (b, "z".into())];
        assert_eq!(
            keys_by_thread(rows),
            vec![(a, strings(&["x", "y"])), (b, strings(&["z"]))]
        );
    }
}
