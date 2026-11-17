//! An empty release backs its member off the thread, on both backends.
//!
//! A claimant that cannot work (its model provider had no balance) claimed a
//! review, acknowledged it and let go a few seconds later with a plain
//! `release_claim`: no decline, no usage, no message. `claim_next` handed it
//! the same thread straight back, 113 times in a day, since the oldest work
//! comes first and ties go to the last holder. Now a plain release whose
//! hold showed no progress (no usage under the lease, no message of the
//! member's own, no result) holds the thread back from that member, and only
//! that member, for 5 minutes, doubling with each empty release in a row up
//! to 6 hours. Progress resets it; a restart's release, a lapse and a decline
//! leave it as it was. The claimer's queue count agrees with `claim_next`
//! throughout, and a second workspace's member touches none of it.

use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations, StoreError};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const LEASE_SECS: i64 = 600;
const FIRST_HOLD_SECS: i64 = 300;
const MAX_HOLD_SECS: i64 = 21_600;

/// The raw pool, to read a member's backoff and to move time along.
enum Pool {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
}

impl Pool {
    /// `(empty_releases, reclaim_after)` of `member` on `thread`, `(0, None)`
    /// when it has no row.
    async fn backoff(&self, thread: ThreadId, member: MemberId) -> (i64, Option<DateTime<Utc>>) {
        match self {
            Pool::Sqlite(p) => sqlx::query(
                "SELECT empty_releases, reclaim_after FROM maidan_thread_workers
                 WHERE thread_id = ? AND member_id = ?",
            )
            .bind(thread.0)
            .bind(member.0)
            .fetch_optional(p)
            .await
            .expect("read backoff")
            .map_or((0, None), |row| {
                (
                    row.get::<i64, _>("empty_releases"),
                    row.get("reclaim_after"),
                )
            }),
            Pool::Postgres(p) => sqlx::query(
                "SELECT empty_releases, reclaim_after FROM maidan_thread_workers
                 WHERE thread_id = $1 AND member_id = $2",
            )
            .bind(thread.0)
            .bind(member.0)
            .fetch_optional(p)
            .await
            .expect("read backoff")
            .map_or((0, None), |row| {
                (
                    i64::from(row.get::<i32, _>("empty_releases")),
                    row.get("reclaim_after"),
                )
            }),
        }
    }

    /// Let `member`'s hold on `thread` run out, as time would, leaving the
    /// count as it is.
    async fn run_out(&self, thread: ThreadId, member: MemberId) {
        let past = Utc::now() - Duration::hours(1);
        match self {
            Pool::Sqlite(p) => {
                sqlx::query(
                    "UPDATE maidan_thread_workers SET reclaim_after = ?
                     WHERE thread_id = ? AND member_id = ? AND reclaim_after IS NOT NULL",
                )
                .bind(past.to_rfc3339())
                .bind(thread.0)
                .bind(member.0)
                .execute(p)
                .await
                .expect("run out");
            }
            Pool::Postgres(p) => {
                sqlx::query(
                    "UPDATE maidan_thread_workers SET reclaim_after = $1
                     WHERE thread_id = $2 AND member_id = $3 AND reclaim_after IS NOT NULL",
                )
                .bind(past)
                .bind(thread.0)
                .bind(member.0)
                .execute(p)
                .await
                .expect("run out");
            }
        }
    }

    /// Move every message on `thread` two hours back, past any hold that
    /// starts now.
    async fn age_messages(&self, thread: ThreadId) {
        let past = Utc::now() - Duration::hours(2);
        match self {
            Pool::Sqlite(p) => {
                sqlx::query("UPDATE maidan_messages SET posted_at = ? WHERE thread_id = ?")
                    .bind(past.to_rfc3339())
                    .bind(thread.0)
                    .execute(p)
                    .await
                    .expect("age messages");
            }
            Pool::Postgres(p) => {
                sqlx::query("UPDATE maidan_messages SET posted_at = $1 WHERE thread_id = $2")
                    .bind(past)
                    .bind(thread.0)
                    .execute(p)
                    .await
                    .expect("age messages");
            }
        }
    }

    /// Lapse the live lease on `thread`, as time would.
    async fn lapse(&self, thread: ThreadId) {
        let past = Utc::now() - Duration::hours(1);
        match self {
            Pool::Sqlite(p) => {
                sqlx::query("UPDATE maidan_threads SET assignment_expires_at = ? WHERE id = ?")
                    .bind(past.to_rfc3339())
                    .bind(thread.0)
                    .execute(p)
                    .await
                    .expect("lapse");
            }
            Pool::Postgres(p) => {
                sqlx::query("UPDATE maidan_threads SET assignment_expires_at = $1 WHERE id = $2")
                    .bind(past)
                    .bind(thread.0)
                    .execute(p)
                    .await
                    .expect("lapse");
            }
        }
    }
}

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    a: MemberId,
    b: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace")
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let mut seats = Vec::new();
    for handle in ["a", "b"] {
        seats.push(
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect("member")
                .id,
        );
    }
    Room {
        ws,
        channel,
        a: seats[0],
        b: seats[1],
    }
}

/// A thread titled `title`; titles differ, or the fingerprint withholds the
/// later one as a duplicate.
async fn task(store: &dyn Store, channel: ChannelId, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id
}

async fn next(store: &dyn Store, r: &Room, member: MemberId) -> Option<Thread> {
    store
        .claim_next_thread(r.channel, member, Some(LEASE_SECS), None)
        .await
        .expect("claim next")
}

fn lease(thread: &Thread) -> ClaimLeaseId {
    thread.claim_lease_id.expect("a leased claim")
}

/// `member` claims `thread`, which must be what `claim_next` hands it, and
/// acknowledges it, as the claimant in the incident did.
async fn take(store: &dyn Store, r: &Room, member: MemberId, thread: ThreadId) -> Thread {
    let held = next(store, r, member).await.expect("a claimable thread");
    assert_eq!(held.id, thread, "claim_next hands the expected thread");
    store
        .acknowledge_claim(held.id, member, lease(&held))
        .await
        .expect("acknowledge");
    held
}

/// A plain release of `held` by `member`, returning the clock read either
/// side of it, so a hold the release set can be bounded.
async fn release(
    store: &dyn Store,
    held: &Thread,
    member: MemberId,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let before = Utc::now();
    store
        .release_claim_with_event(held.id, member, lease(held), None)
        .await
        .expect("plain release");
    (before, Utc::now())
}

/// The hold `reclaim_after` sets lasts `secs` from the release, give or take
/// the time the release took.
fn assert_hold(
    reclaim_after: Option<DateTime<Utc>>,
    (before, after): (DateTime<Utc>, DateTime<Utc>),
    secs: i64,
    what: &str,
) {
    let at = reclaim_after.unwrap_or_else(|| panic!("{what}: no reclaim_after"));
    // A millisecond either side: SQLite compares at millisecond grain.
    let low = before + Duration::seconds(secs) - Duration::milliseconds(1);
    let high = after + Duration::seconds(secs) + Duration::milliseconds(1);
    assert!(
        low <= at && at <= high,
        "{what}: reclaim_after {at} is not {secs}s after the release ({before}..{after})"
    );
}

async fn ready(store: &dyn Store, r: &Room, member: MemberId) -> i64 {
    store
        .workspace_queue_depth(r.ws, member, None, None, None, true)
        .await
        .expect("depth")
        .ready
}

async fn say(store: &dyn Store, thread: ThreadId, who: MemberId) {
    store
        .post_message(NewMessage {
            thread_id: thread,
            author_id: who,
            body: "working on it".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("message");
}

fn usage(thread: ThreadId, reporter: MemberId, lease: ClaimLeaseId) -> NewUsageLedgerEntry {
    NewUsageLedgerEntry {
        usage_report_id: uuid::Uuid::now_v7(),
        thread_id: thread,
        reporter,
        claim_lease_id: lease,
        model: "provider/model".into(),
        tokens: TokenUsage {
            input: 10,
            ..Default::default()
        },
        usd_micros: 0,
        price_snapshot: PriceSnapshot::default(),
        turns: 1,
        evidence: Default::default(),
    }
}

/// 1 and 6: the empty release holds the thread back from its member only,
/// and that member's queue count agrees with what `claim_next` hands it.
async fn an_empty_release_holds_the_thread_back_from_that_member_only(
    store: &dyn Store,
    pool: &Pool,
) {
    let r = room(store, "empty").await;
    let stuck = task(store, r.channel, "stuck").await;
    let spare = task(store, r.channel, "spare").await;
    assert_eq!(ready(store, &r, r.a).await, 2);

    let held = take(store, &r, r.a, stuck).await;
    let window = release(store, &held, r.a).await;
    let (count, reclaim_after) = pool.backoff(stuck, r.a).await;
    assert_eq!(count, 1, "one empty release");
    assert_hold(reclaim_after, window, FIRST_HOLD_SECS, "the first hold");
    assert_eq!(
        pool.backoff(stuck, r.b).await,
        (0, None),
        "nobody else is backed off"
    );

    assert_eq!(
        ready(store, &r, r.a).await,
        1,
        "the member's count leaves out the thread it let go of"
    );
    assert_eq!(
        ready(store, &r, r.b).await,
        2,
        "every other member's still holds it"
    );
    let got = next(store, &r, r.a).await.expect("the count promised one");
    assert_eq!(got.id, spare, "the next thread, not the one it let go of");
    assert_eq!(ready(store, &r, r.a).await, 0, "and nothing is left for it");
    assert!(next(store, &r, r.a).await.is_none(), "claim_next agrees");
    let (across, _) = store
        .claim_next_workspace_thread_with_event(r.ws, r.a, Some(LEASE_SECS), None, None, None, None)
        .await
        .expect("claim next across the workspace");
    assert!(across.is_none(), "the workspace scope holds it back too");

    let taken = next(store, &r, r.b).await.expect("another member");
    assert_eq!(taken.id, stuck, "every other member is handed it at once");
}

/// 2: once `reclaim_after` passes, the member is handed the thread again.
async fn the_member_is_handed_the_thread_again_once_the_hold_runs_out(
    store: &dyn Store,
    pool: &Pool,
) {
    let r = room(store, "runs-out").await;
    let only = task(store, r.channel, "only").await;
    let held = take(store, &r, r.a, only).await;
    release(store, &held, r.a).await;
    assert!(next(store, &r, r.a).await.is_none(), "held back");
    assert_eq!(ready(store, &r, r.a).await, 0);

    pool.run_out(only, r.a).await;
    assert_eq!(ready(store, &r, r.a).await, 1, "the count has it again");
    let again = next(store, &r, r.a).await.expect("handed again");
    assert_eq!(again.id, only);
}

/// 3: each empty release in a row doubles the hold, from 5 minutes up to its
/// 6-hour cap.
async fn consecutive_empty_releases_double_the_hold_up_to_its_cap(store: &dyn Store, pool: &Pool) {
    let r = room(store, "doubling").await;
    let only = task(store, r.channel, "only").await;
    let mut expected = FIRST_HOLD_SECS;
    for n in 1..=9 {
        let held = take(store, &r, r.a, only).await;
        let window = release(store, &held, r.a).await;
        let (count, reclaim_after) = pool.backoff(only, r.a).await;
        assert_eq!(count, n, "empty release {n}");
        assert_hold(reclaim_after, window, expected, &format!("hold {n}"));
        assert!(next(store, &r, r.a).await.is_none(), "held back after {n}");
        pool.run_out(only, r.a).await;
        expected = (expected * 2).min(MAX_HOLD_SECS);
    }
    assert_eq!(expected, MAX_HOLD_SECS, "the run reached the cap");
}

/// 4: a release after a usage report, the member's own message or a stored
/// result does not back off, and resets the count; another member's message
/// or one from before the hold is not this hold's progress.
async fn a_release_that_showed_progress_resets_the_backoff(store: &dyn Store, pool: &Pool) {
    let r = room(store, "progress").await;
    let only = task(store, r.channel, "only").await;
    for n in 1..=2 {
        let held = take(store, &r, r.a, only).await;
        release(store, &held, r.a).await;
        assert_eq!(pool.backoff(only, r.a).await.0, n);
        pool.run_out(only, r.a).await;
    }

    // A usage report under the claim.
    let held = take(store, &r, r.a, only).await;
    store
        .report_accounted_usage(&usage(only, r.a, lease(&held)))
        .await
        .expect("usage");
    release(store, &held, r.a).await;
    assert_eq!(
        pool.backoff(only, r.a).await,
        (0, None),
        "usage resets the count and clears the hold"
    );
    // The usage was the last claim's, so this hold is empty, and the count
    // starts again from the first step.
    let held = take(store, &r, r.a, only).await;
    let window = release(store, &held, r.a).await;
    let (count, reclaim_after) = pool.backoff(only, r.a).await;
    assert_eq!(count, 1, "the count restarted");
    assert_hold(reclaim_after, window, FIRST_HOLD_SECS, "after a reset");
    pool.run_out(only, r.a).await;

    // Another member's message is not this member's progress.
    let held = take(store, &r, r.a, only).await;
    say(store, only, r.b).await;
    release(store, &held, r.a).await;
    assert_eq!(pool.backoff(only, r.a).await.0, 2, "b's word is not a's");
    pool.run_out(only, r.a).await;

    // The member's own message is.
    let held = take(store, &r, r.a, only).await;
    say(store, only, r.a).await;
    release(store, &held, r.a).await;
    assert_eq!(pool.backoff(only, r.a).await, (0, None), "its own message");
    let again = next(store, &r, r.a)
        .await
        .expect("a release with progress holds nothing back");
    assert_eq!(again.id, only);

    // A message from before the hold says nothing about it.
    pool.age_messages(only).await;
    store
        .acknowledge_claim(only, r.a, lease(&again))
        .await
        .expect("acknowledge");
    release(store, &again, r.a).await;
    assert_eq!(
        pool.backoff(only, r.a).await.0,
        1,
        "an old message is not progress"
    );
    pool.run_out(only, r.a).await;

    // A stored result is progress too. It also finishes the thread, so
    // nothing is handed out after it either way.
    let held = take(store, &r, r.a, only).await;
    store
        .set_thread_result(only, r.a, lease(&held), &serde_json::json!({"done": true}))
        .await
        .expect("result");
    release(store, &held, r.a).await;
    assert_eq!(pool.backoff(only, r.a).await, (0, None), "a result");
}

/// 5: a restart's release of every claim, a lapse taken over by `claim_next`
/// or the reaper, and a decline leave the backoff as it was.
async fn a_restart_a_lapse_and_a_decline_leave_the_backoff_alone(store: &dyn Store, pool: &Pool) {
    let r = room(store, "untouched").await;
    let first = task(store, r.channel, "first").await;
    let held = take(store, &r, r.a, first).await;
    release(store, &held, r.a).await;
    pool.run_out(first, r.a).await;
    let before = pool.backoff(first, r.a).await;
    assert_eq!(before.0, 1);

    // A restart releases what the old process held, and judges none of it.
    take(store, &r, r.a, first).await;
    let released = store
        .release_member_claims_with_events(r.ws, r.a, None)
        .await
        .expect("release own claims");
    assert_eq!(released.len(), 1);
    assert_eq!(pool.backoff(first, r.a).await, before, "a restart");

    // A lapse taken over by another member's claim.
    take(store, &r, r.a, first).await;
    pool.lapse(first).await;
    let taken = next(store, &r, r.b).await.expect("the lapsed claim");
    assert_eq!(taken.id, first);
    assert_eq!(pool.backoff(first, r.a).await, before, "a takeover");
    assert_eq!(pool.backoff(first, r.b).await, (0, None));
    store
        .release_claim_with_event(first, r.b, lease(&taken), Some(ClaimDecline::Unworkable))
        .await
        .expect("b declines");

    // A lapse the reaper frees.
    take(store, &r, r.a, first).await;
    pool.lapse(first).await;
    store
        .reap_expired_claims(Utc::now(), 100)
        .await
        .expect("reap");
    assert_eq!(pool.backoff(first, r.a).await, before, "a reaped lapse");

    // A decline, which withholds the thread for good.
    let held = take(store, &r, r.a, first).await;
    store
        .release_claim_with_event(first, r.a, lease(&held), Some(ClaimDecline::Unworkable))
        .await
        .expect("decline");
    assert_eq!(pool.backoff(first, r.a).await, before, "a decline");
}

/// Two tenants: another workspace's member cannot release a claim it does not
/// hold, so it records no backoff, and its own empty release moves nothing in
/// the first workspace.
async fn another_workspace_neither_sets_nor_feels_the_backoff(store: &dyn Store, pool: &Pool) {
    let mine = room(store, "mine").await;
    let theirs = room(store, "theirs").await;
    let here = task(store, mine.channel, "here").await;
    let there = task(store, theirs.channel, "there").await;

    let held = take(store, &mine, mine.a, here).await;
    let refused = store
        .release_claim_with_event(here, theirs.a, lease(&held), None)
        .await;
    assert!(
        matches!(refused, Err(StoreError::ClaimLost(_))),
        "a stranger's release is the claim fence's, got {refused:?}"
    );
    assert_eq!(pool.backoff(here, mine.a).await, (0, None));
    assert_eq!(pool.backoff(here, theirs.a).await, (0, None));
    assert_eq!(
        store.get_thread(here).await.expect("thread").assignee_id,
        Some(mine.a),
        "the holder keeps it"
    );

    let other = take(store, &theirs, theirs.a, there).await;
    release(store, &other, theirs.a).await;
    assert_eq!(pool.backoff(there, theirs.a).await.0, 1);
    assert_eq!(
        pool.backoff(here, mine.a).await,
        (0, None),
        "the other workspace's empty release moves nothing here"
    );
    assert!(
        next(store, &theirs, theirs.a).await.is_none(),
        "its member is backed off its own thread and sees none of ours"
    );
    assert_eq!(
        next(store, &theirs, theirs.b).await.map(|t| t.id),
        Some(there)
    );
    assert_eq!(ready(store, &mine, mine.b).await, 0, "ours is still held");
}

async fn run_suite(store: &dyn Store, pool: &Pool) {
    an_empty_release_holds_the_thread_back_from_that_member_only(store, pool).await;
    the_member_is_handed_the_thread_again_once_the_hold_runs_out(store, pool).await;
    consecutive_empty_releases_double_the_hold_up_to_its_cap(store, pool).await;
    a_release_that_showed_progress_resets_the_backoff(store, pool).await;
    a_restart_a_lapse_and_a_decline_leave_the_backoff_alone(store, pool).await;
    another_workspace_neither_sets_nor_feels_the_backoff(store, pool).await;
}

#[tokio::test]
async fn an_empty_release_backs_the_member_off_on_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    run_suite(&store, &Pool::Sqlite(pool)).await;
}

#[tokio::test]
async fn an_empty_release_backs_the_member_off_on_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    run_suite(&store, &Pool::Postgres(pool)).await;
}
