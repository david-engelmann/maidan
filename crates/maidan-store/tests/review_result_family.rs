//! A review's holder is judged again when it writes the review's result
//! (#1703), on both backends. Every claim-time check reads a seat's family
//! from usage it reported before it was handed the review, so a seat with
//! none is handed its own family's review. Its result is then refused until
//! it reports usage on the review, and refused for good once that usage
//! names the author's family; the usage itself is accepted and the claim
//! stays held, so it can decline. A seat of another family, and a person,
//! complete the same review. Usage filed under another workspace counts for
//! nothing.

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const LEASE_SECS: i64 = 600;

/// One usage record written straight to the ledger, filed under `ws`
/// whatever workspace `thread` is in: the shape no report through the store
/// can take, used to show the rule reads only the thread's own workspace.
struct Filed {
    ws: Uuid,
    thread: Uuid,
    reporter: Uuid,
    model: &'static str,
}

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
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
            name: "reviews".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    Room { ws, channel }
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str, kind: MemberKind) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind,
        })
        .await
        .expect("member")
        .id
}

/// A review of a change `author` wrote (titles differ, or the fingerprint
/// withholds the later one as a duplicate).
async fn review(store: &dyn Store, channel: ChannelId, title: &str, author: &str) -> ThreadId {
    let id = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id;
    store
        .set_thread_attributes(
            id,
            TaskAttributes {
                kind: TaskKind::Review,
                author_family: Some(author.into()),
                ..TaskAttributes::default()
            },
        )
        .await
        .expect("review attributes");
    id
}

async fn next(store: &dyn Store, channel: ChannelId, who: MemberId) -> Option<Thread> {
    store
        .claim_next_thread(channel, who, Some(LEASE_SECS), None)
        .await
        .expect("claim next")
}

fn lease(thread: &Thread) -> ClaimLeaseId {
    thread.claim_lease_id.expect("a leased claim")
}

/// What a seat sends `report_usage` with: zero tokens, and the evidence that
/// names its provider.
fn usage(
    thread: ThreadId,
    reporter: MemberId,
    lease: ClaimLeaseId,
    model: &str,
    provider: &str,
) -> NewUsageLedgerEntry {
    NewUsageLedgerEntry {
        usage_report_id: Uuid::now_v7(),
        thread_id: thread,
        reporter,
        claim_lease_id: lease,
        model: model.into(),
        tokens: TokenUsage::default(),
        usd_micros: 0,
        price_snapshot: PriceSnapshot::default(),
        turns: 1,
        evidence: UsageEvidence {
            provider: Some(provider.into()),
            harness: Some("test".into()),
            ..UsageEvidence::default()
        },
    }
}

fn verdict() -> serde_json::Value {
    json!({"status": "done", "summary": "VERDICT: PASS"})
}

async fn run_suite<F, Fut>(store: &dyn Store, file: F)
where
    F: Fn(Filed) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let w = room(store, "reviews").await;
    let fresh = member(store, w.ws, "fresh", MemberKind::Agent).await;
    let other = member(store, w.ws, "other", MemberKind::Agent).await;
    let person = member(store, w.ws, "person", MemberKind::Human).await;
    let first = review(store, w.channel, "first review", "anthropic").await;
    let second = review(store, w.channel, "second review", "anthropic").await;

    // `fresh` has reported nothing, so no claim-time check can tell its
    // family: it is handed the anthropic review.
    let held = next(store, w.channel, fresh).await.expect("a review");
    assert_eq!(held.id, first);
    let lease_held = lease(&held);

    // A verdict before any usage is refused: nothing yet says which family
    // did the review.
    let early = store
        .set_thread_result(first, fresh, lease_held, &verdict())
        .await;
    assert!(
        matches!(&early, Err(StoreError::Conflict(m)) if m.contains("reported no usage")),
        "{early:?}"
    );

    // The usage naming the author's family is accepted: the spend stays on
    // the ledger and the budget.
    let (entry, _) = store
        .report_accounted_usage(&usage(
            first,
            fresh,
            lease_held,
            "claude-opus-4-5",
            "anthropic",
        ))
        .await
        .expect("the usage report is accepted");
    assert_eq!(entry.stamp.reporter, fresh);
    assert!(!entry.stopped);

    // Its verdict is refused, naming the family and the way out; nothing is
    // stored, so nothing is delivered, and the claim stays held.
    let refused = store
        .set_thread_result(first, fresh, lease_held, &verdict())
        .await;
    assert!(
        matches!(&refused, Err(StoreError::Conflict(m))
            if m.contains("anthropic family") && m.contains("decline own_family")),
        "{refused:?}"
    );
    assert!(
        store
            .get_thread_result(first)
            .await
            .expect("read result")
            .is_none(),
        "a same-family verdict is never stored"
    );
    let still = store.get_thread(first).await.expect("thread");
    assert_eq!(still.assignee_id, Some(fresh), "the claim stays held");
    assert_eq!(still.claim_lease_id, Some(lease_held));
    store
        .release_claim_with_event(first, fresh, lease_held, Some(ClaimDecline::OwnFamily))
        .await
        .expect("the holder declines it");

    // The usage it reported now refuses it the other anthropic review up
    // front.
    assert!(
        next(store, w.channel, fresh).await.is_none(),
        "its usage names its family now"
    );

    // A second workspace. Anthropic usage `other` has filed there counts for
    // nothing here, and its seat cannot write a verdict on a review it does
    // not hold: it learns only that, nothing about the family.
    let w2 = room(store, "elsewhere").await;
    let stranger = member(store, w2.ws, "fresh", MemberKind::Agent).await;
    let theirs = review(store, w2.channel, "their review", "anthropic").await;
    file(Filed {
        ws: w2.ws.0,
        thread: theirs.0,
        reporter: other.0,
        model: "claude-opus-4",
    })
    .await;

    // A seat of another family completes the same review the same way.
    let taken = next(store, w.channel, other)
        .await
        .expect("another family takes it");
    assert_eq!(taken.id, first);
    let foreign = store
        .set_thread_result(first, stranger, lease(&taken), &verdict())
        .await;
    assert!(
        matches!(&foreign, Err(StoreError::ClaimLost(_))),
        "a non-holder is answered by the claim fence, got {foreign:?}"
    );
    store
        .report_accounted_usage(&usage(first, other, lease(&taken), "gpt-5-codex", "openai"))
        .await
        .expect("usage");
    let stored = store
        .set_thread_result(first, other, lease(&taken), &verdict())
        .await
        .expect("another family's verdict is stored");
    assert_eq!(stored.produced_by, other);
    let read = store
        .get_thread_result(first)
        .await
        .expect("read result")
        .expect("a result");
    assert_eq!(read.produced_by, other);

    // A person is no model family and reports no usage: their verdict is
    // stored.
    let by_hand = store
        .claim_thread(second, person)
        .await
        .expect("the person takes the other review");
    assert!(by_hand.claimed);
    store
        .set_thread_result(second, person, lease(&by_hand.thread), &verdict())
        .await
        .expect("a person's verdict is stored");

    // Usage on this review filed under another workspace is not usage on
    // it: the seat is still asked to report.
    let third = review(store, w.channel, "third review", "anthropic").await;
    let late = member(store, w.ws, "late", MemberKind::Agent).await;
    let held = next(store, w.channel, late).await.expect("a review");
    assert_eq!(held.id, third);
    file(Filed {
        ws: w2.ws.0,
        thread: third.0,
        reporter: late.0,
        model: "gpt-5",
    })
    .await;
    let unreported = store
        .set_thread_result(third, late, lease(&held), &verdict())
        .await;
    assert!(
        matches!(&unreported, Err(StoreError::Conflict(m)) if m.contains("reported no usage")),
        "{unreported:?}"
    );

    // The rule is the review's: the second workspace's seat, with no usage
    // anywhere, is asked to report on its own review as well.
    let own = next(store, w2.channel, stranger)
        .await
        .expect("its own review");
    assert_eq!(own.id, theirs);
    let asked = store
        .set_thread_result(theirs, stranger, lease(&own), &verdict())
        .await;
    assert!(
        matches!(&asked, Err(StoreError::Conflict(m)) if m.contains("reported no usage")),
        "{asked:?}"
    );

    // A thread that is not a family-routed review is untouched by the rule.
    let plain = store
        .create_thread(NewThread {
            channel_id: w.channel,
            parent_thread_id: None,
            title: Some("plain work".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id;
    let work = store.claim_thread(plain, fresh).await.expect("claim");
    store
        .set_thread_result(plain, fresh, lease(&work.thread), &verdict())
        .await
        .expect("plain work takes a result without usage");
}

const INSERT_SQLITE: &str = "INSERT INTO maidan_usage_ledger
    (usage_report_id, workspace_id, thread_id, reporter_id, claim_lease_id, model,
     input_tokens, output_tokens, cache_read_tokens, cache_write_5m_tokens,
     input_price, output_price, cache_read_price, cache_write_5m_price, usd_micros, turns)
    VALUES (?, ?, ?, ?, ?, ?, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)";

const INSERT_POSTGRES: &str = "INSERT INTO maidan_usage_ledger
    (usage_report_id, workspace_id, thread_id, reporter_id, claim_lease_id, model,
     input_tokens, output_tokens, cache_read_tokens, cache_write_5m_tokens,
     input_price, output_price, cache_read_price, cache_write_5m_price, usd_micros, turns)
    VALUES ($1, $2, $3, $4, $5, $6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)";

#[tokio::test]
async fn a_review_holder_whose_usage_names_the_authors_family_cannot_store_its_verdict_on_sqlite() {
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
    run_suite(&store, |f| {
        let pool = pool.clone();
        async move {
            sqlx::query(INSERT_SQLITE)
                .bind(Uuid::now_v7())
                .bind(f.ws)
                .bind(f.thread)
                .bind(f.reporter)
                .bind(Uuid::now_v7())
                .bind(f.model)
                .execute(&pool)
                .await
                .expect("filed usage");
        }
    })
    .await;
}

#[tokio::test]
async fn a_review_holder_whose_usage_names_the_authors_family_cannot_store_its_verdict_on_postgres()
{
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
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    run_suite(&store, |f| {
        let pool = pool.clone();
        async move {
            sqlx::query(INSERT_POSTGRES)
                .bind(Uuid::now_v7())
                .bind(f.ws)
                .bind(f.thread)
                .bind(f.reporter)
                .bind(Uuid::now_v7())
                .bind(f.model)
                .execute(&pool)
                .await
                .expect("filed usage");
        }
    })
    .await;
}
