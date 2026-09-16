//! Cluster 392: hash chain on append; tamper is detected. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    content_hash, genesis_hash, ChainBreakReason, Event, MemberKind, NewChannel, NewMember,
    NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn seed(store: &dyn Store) -> (maidan_types::WorkspaceId, maidan_types::Member) {
    // Plain `create_*` does not append to the log. The chain is only
    // written by `append` / `*_with_event`.
    let (ws, stored_ws) = store
        .create_workspace_with_event(NewWorkspace {
            name: "chain-ws".into(),
        })
        .await
        .expect("ws");
    assert_eq!(stored_ws.prev_hash, genesis_hash());
    assert_eq!(stored_ws.lsn, stored_ws.id);
    let (member, stored_member) = store
        .create_member_with_event(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    assert_ne!(stored_member.prev_hash, genesis_hash());
    let _ch = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    (ws.id, member)
}

async fn run_suite(store: &dyn Store) {
    let report = store
        .verify_event_chain(maidan_types::WorkspaceId(uuid::Uuid::nil()))
        .await
        .expect("empty unknown ws");
    assert!(report.ok);
    assert_eq!(report.checked, 0);

    let (ws, member) = seed(store).await;
    let report = store.verify_event_chain(ws).await.expect("after seed");
    assert!(report.ok, "{report:?}");
    assert!(report.checked >= 2, "workspace + member (+ channel) events");
    assert!(report.from_genesis);
    let head = report.head.expect("head");
    assert_eq!(head.lsn, head.id);
    assert!(head.prev_hash.starts_with("sha256:"));
    assert!(head.content_hash.starts_with("sha256:"));

    let extra = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member: member.clone(),
        })
        .await
        .expect("append");
    assert_eq!(extra.lsn, extra.id);
    assert_ne!(extra.prev_hash, genesis_hash());
    assert_eq!(
        extra.content_hash,
        content_hash(&extra.payload).expect("hash")
    );
    let report = store.verify_event_chain(ws).await.expect("after append");
    assert!(report.ok, "{report:?}");
    assert_eq!(report.head.as_ref().map(|h| h.id), Some(extra.id));

    let (ws_b, stored_b) = store
        .create_workspace_with_event(NewWorkspace {
            name: "other".into(),
        })
        .await
        .expect("ws b");
    let report_b = store.verify_event_chain(ws_b.id).await.expect("b");
    assert!(report_b.ok);
    assert!(report_b.from_genesis);
    assert_eq!(report_b.checked, 1);
    assert_eq!(stored_b.prev_hash, genesis_hash());
    // Other-tenant appends must not fork this workspace's chain.
    let still = store.verify_event_chain(ws).await.expect("a isolated");
    assert!(still.ok);
    assert_eq!(still.head.as_ref().map(|h| h.id), Some(extra.id));
}

#[tokio::test]
async fn append_then_verify_ok_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn tamper_is_detected_sqlite() {
    let store = sqlite().await;
    let (ws, member) = seed(&store).await;
    let stored = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member,
        })
        .await
        .expect("append");
    assert!(store.verify_event_chain(ws).await.expect("pre").ok);

    let mut payload = stored.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(stored.id)
        .execute(store.pool())
        .await
        .expect("tamper");

    let report = store.verify_event_chain(ws).await.expect("verify");
    assert!(!report.ok);
    assert_eq!(report.break_at, Some(stored.id));
    assert_eq!(report.reason, Some(ChainBreakReason::ContentHashMismatch));
}

#[tokio::test]
async fn prev_hash_break_is_detected_sqlite() {
    let store = sqlite().await;
    let (ws, member) = seed(&store).await;
    let stored = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member,
        })
        .await
        .expect("append");
    sqlx::query("UPDATE maidan_events SET prev_hash = ? WHERE id = ?")
        .bind(genesis_hash())
        .bind(stored.id)
        .execute(store.pool())
        .await
        .expect("tamper prev");
    let report = store.verify_event_chain(ws).await.expect("verify");
    assert!(!report.ok);
    assert_eq!(report.break_at, Some(stored.id));
    assert_eq!(report.reason, Some(ChainBreakReason::PrevHashMismatch));
}

#[tokio::test]
async fn append_then_verify_ok_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}

#[tokio::test]
async fn tamper_is_detected_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    let (ws, member) = seed(&store).await;
    let stored = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member,
        })
        .await
        .expect("append");
    assert!(store.verify_event_chain(ws).await.expect("pre").ok);

    let mut payload = stored.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = $1 WHERE id = $2")
        .bind(&payload)
        .bind(stored.id)
        .execute(store.pool())
        .await
        .expect("tamper");

    let report = store.verify_event_chain(ws).await.expect("verify");
    assert!(!report.ok);
    assert_eq!(report.break_at, Some(stored.id));
    assert_eq!(report.reason, Some(ChainBreakReason::ContentHashMismatch));
}

/// Cluster 397.8: a blanked `content_hash` must not launder a tampered payload.
///
/// `backfill_chain` runs on every startup. Its guard used to be global — if *any*
/// row had an empty `content_hash`, it re-linked **every row of every workspace**
/// from genesis against the current payloads. So the attack against a
/// tamper-evident log was: edit a payload, blank one row's hash, restart, and the
/// chain is recomputed to agree with you. `verify_event_chain` then said
/// `ok: true, from_genesis: true`.
///
/// Now backfill only fills rows that are actually empty and never rewrites a row
/// that already has a hash, so the successor's `prev_hash` — still chaining from
/// the original — no longer matches, and the break surfaces.
#[tokio::test]
async fn a_blanked_hash_cannot_launder_a_tampered_payload_sqlite() {
    let store = sqlite().await;
    let (ws, member) = seed(&store).await;

    let first = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member: member.clone(),
        })
        .await
        .expect("append 1");
    let second = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member,
        })
        .await
        .expect("append 2");
    assert!(store.verify_event_chain(ws).await.expect("pre").ok);

    // The attack: rewrite the first event's payload, then blank its hash so the
    // startup backfill treats it as an un-hashed legacy row.
    let mut payload = first.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = ?, content_hash = '' WHERE id = ?")
        .bind(payload.to_string())
        .bind(first.id)
        .execute(store.pool())
        .await
        .expect("tamper + blank");

    // Restart: migrations (and so the backfill) run again.
    maidan_store::run_sqlite_migrations(store.pool())
        .await
        .expect("re-run migrations");

    let report = store.verify_event_chain(ws).await.expect("verify");
    assert!(
        !report.ok,
        "the tamper must still be visible after a backfill pass; got {report:?}"
    );
    assert_eq!(
        report.break_at,
        Some(second.id),
        "the break surfaces at the successor, whose prev_hash still names the original"
    );
}

/// Cluster 400.3: a payload carrying exponent-notation numbers still verifies.
///
/// Postgres `jsonb` parses each number into `numeric` and re-renders it, which
/// **expands exponent notation** — `1E2` is stored as `100`. An in-memory `1e2`
/// hashes as the float `100.0`; the stored `100` reads back as an *integer* and
/// hashes differently, so `verify_event_chain` reported a tamper on an event
/// nobody had touched, permanently, for that workspace.
///
/// Reachable from ordinary use: message `metadata` is arbitrary client JSON and
/// `JSON.stringify` emits exponents above `1e21`.
///
/// Postgres only — SQLite stores the payload text verbatim, so the round trip
/// that causes this does not exist there. The normalization runs on both
/// backends anyway, because a federated origin hash computed on one must verify
/// on the other.
async fn assert_exponent_numbers_survive_the_round_trip(store: &dyn Store) {
    let (ws, member) = seed(store).await;

    // Every shape jsonb rewrites, plus the ones it leaves alone, so a future
    // change to the normalizer that over-reaches fails here.
    let payload = serde_json::json!({
        // The window that actually breaks: serde renders an `f64` with an
        // exponent from 1e16 up, and jsonb expands that to a plain integer that
        // still fits `u64` below ~1.8e19 — so it reads back as an *integer* and
        // hashes differently from the float that was hashed on the way in.
        "at_the_boundary": 1e16,
        "inside_the_window": 5e18,
        "nested_in_window": [1e17, {"deep": 2e16}],
        // Below the window serde writes plain decimal, which jsonb preserves.
        "below_window": 1e15,
        "small_integral": 1e2,
        // Above it the expansion overflows `u64`, so both sides fall back to
        // `f64` and already agree.
        "above_window": 1e30,
        // Untouched shapes — a normalizer that over-reaches fails here.
        "small_exponent": 1e-7,
        "fractional": 0.1,
        "trailing_zero": 1.10,
        "plain_integer": 3,
        "big_integer": 9007199254740993i64,
        "negative_in_window": -5e18,
    });

    // Appended the way **federation ingest** does it: a peer's JSON is parsed
    // into an `Event` and published straight to the log. That is the reachable
    // path, and the distinction matters — a *local* post is laundered first,
    // because the message row is inserted into a `jsonb` column and the event
    // is built from what came back, so both sides already agree. Only an event
    // that reaches the log without a prior round trip can disagree with itself.
    let channel = store.list_channels(ws).await.expect("channels")[0].id;
    let thread = store
        .create_thread(maidan_types::NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("numbers".into()),
        })
        .await
        .expect("thread");
    let now = chrono::Utc::now();
    let stored = store
        .append_event(&Event::MessagePosted {
            occurred_at: now,
            workspace_id: ws,
            channel_id: channel,
            thread_id: thread.id,
            dm_conversation_id: None,
            message: maidan_types::Message {
                id: maidan_types::MessageId::new(),
                thread_id: thread.id,
                author_id: member.id,
                body: "numbers".into(),
                metadata: payload.clone(),
                content: None,
                posted_at: now,
                edited_at: None,
                tombstoned_at: None,
            },
        })
        .await
        .expect("append with exponent numbers");

    let report = store.verify_event_chain(ws).await.expect("verify");
    assert!(
        report.ok,
        "an untouched event with exponent numbers must verify: {report:?}"
    );

    // And the stored payload must still mean the same thing — normalizing is
    // allowed to change a number's spelling, never its value.
    let read_back = store
        .list_events_after(ws, stored.id - 1, 1)
        .await
        .expect("read back");
    let got = &read_back[0].payload["message"]["metadata"];
    assert_eq!(
        got["at_the_boundary"],
        serde_json::json!(10_000_000_000_000_000u64)
    );
    assert_eq!(
        got["inside_the_window"],
        serde_json::json!(5_000_000_000_000_000_000u64)
    );
    assert_eq!(
        got["negative_in_window"],
        serde_json::json!(-5_000_000_000_000_000_000i64)
    );
    assert_eq!(
        got["fractional"],
        serde_json::json!(0.1),
        "a non-integral value must be left alone"
    );
    assert_eq!(
        got["big_integer"],
        serde_json::json!(9007199254740993i64),
        "an integer past 2^53 must never be routed through f64"
    );
}

#[tokio::test]
async fn exponent_numbers_survive_the_jsonb_round_trip_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    assert_exponent_numbers_survive_the_round_trip(&store).await;
}

#[tokio::test]
async fn exponent_numbers_survive_the_round_trip_sqlite() {
    let store = sqlite().await;
    assert_exponent_numbers_survive_the_round_trip(&store).await;
}
