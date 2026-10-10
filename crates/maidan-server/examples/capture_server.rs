//! Seed-and-serve harness for the README and listing screenshots.
//!
//! This is **test support, not a shipped binary**. It stands up the real
//! `maidan-server` router on an in-memory SQLite store and seeds the
//! workspaces the screenshot bar in `docs/UI Design.md` asks for: a board
//! with work in flight, the same board with a review waiting on the signed-in
//! person (whose agent was refused when it tried to close its own work), and
//! a channel with no tasks yet. `ui-tests/capture/capture.spec.ts` drives the
//! real `/ui` against it (`npm run capture` in `ui-tests/`).
//!
//! The pictures must come out the same on every run, so everything the page
//! draws is pinned: member ids (an avatar's hue comes from its id), and every
//! timestamp, which is moved onto a fixed clock (`CAPTURE_NOW`) that the
//! browser is also set to. Nothing here runs the claim reaper, so a lease
//! that has lapsed by the wall clock still reads as held.
//!
//! Env: `CAPTURE_PORT` (default 8961), `CAPTURE_FIXTURES` (default
//! `ui-tests/.capture.json`), and `CAPTURE_DEBUG` to print every timestamp
//! the seed left within two minutes of the capture clock. Run via `cargo run
//! --example capture_server`.

use std::net::SocketAddr;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_fsm::ThreadAction;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ArtifactKind, ChannelId, MemberId, MemberKind, NewApiToken, NewArtifact, NewChannel, NewMember,
    NewMessage, NewThread, NewWorkspace, ReviewDecision, ThreadId, WorkspaceId,
};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

const SESSION_SECRET: &[u8] = b"capture-session-secret-at-least-32-bytes";

/// The moment every screenshot is taken at: Tuesday 2026-10-06, 2:40 PM in
/// New York. The browser's clock is set to the same instant.
const CAPTURE_NOW: &str = "2026-10-06T18:40:00Z";

/// Tables whose timestamps stay on the wall clock: a session or token that
/// expired by the fixed clock would refuse the browser, and the event log and
/// audit are hash-chained.
const KEEP_WALL_CLOCK: &[&str] = &["session", "token", "event", "audit", "idempot"];

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(CAPTURE_NOW)
        .expect("CAPTURE_NOW")
        .with_timezone(&Utc)
}

fn mins(m: i64) -> DateTime<Utc> {
    now() - Duration::minutes(m)
}

/// A fixed member id, so the avatar hue (hashed from the id) never changes.
fn fixed_id(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0192_6a00_0000_7000_8000_0000_0000_0000 | n)
}

/// A timestamp to write after seeding: `table.column` of the rows whose `key`
/// column is `id`.
struct Pin {
    table: &'static str,
    key: &'static str,
    column: &'static str,
    id: uuid::Uuid,
    at: DateTime<Utc>,
}

struct Cast {
    david: MemberId,
    planner: MemberId,
    server: MemberId,
    ui: MemberId,
    tester: MemberId,
}

struct Board {
    workspace: WorkspaceId,
    channel: ChannelId,
    cast: Cast,
    review: Option<ThreadId>,
    david_token: String,
    planner_token: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    /// Work in flight; nothing waits on David.
    Flight,
    /// The review names David, and the agent's own close was refused.
    Waiting,
    /// One channel, no tasks.
    Empty,
}

/// Give a row just created a fixed id: every `id` and `*_id` column holding
/// `old` takes `new`, with foreign keys off for the swap. Run before anything
/// else refers to the row, so the only references are the ones its own
/// create wrote.
async fn repin(pool: &SqlitePool, old: uuid::Uuid, new: uuid::Uuid) {
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await
        .expect("foreign keys off");
    let tables: Vec<String> =
        sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'maidan_%'")
            .fetch_all(&mut *conn)
            .await
            .expect("tables")
            .iter()
            .map(|r| r.get::<String, _>(0))
            .collect();
    for table in tables {
        let cols: Vec<String> = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(&mut *conn)
            .await
            .expect("columns")
            .iter()
            .map(|r| r.get::<String, _>("name"))
            .filter(|c| c == "id" || c.ends_with("_id"))
            .collect();
        for col in cols {
            sqlx::query(&format!("UPDATE {table} SET {col} = ? WHERE {col} = ?"))
                .bind(new)
                .bind(old)
                .execute(&mut *conn)
                .await
                .expect("repin id");
        }
    }
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .expect("foreign keys on");
}

async fn member(
    store: &Arc<dyn Store>,
    pool: &SqlitePool,
    ws: WorkspaceId,
    n: u128,
    handle: &str,
    name: &str,
    kind: MemberKind,
) -> MemberId {
    let m = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: Some(name.into()),
            kind,
        })
        .await
        .expect("member");
    let id = fixed_id(n);
    repin(pool, m.id.0, id).await;
    MemberId(id)
}

/// A task the planner files and owns, with its brief as the first message.
#[allow(clippy::too_many_arguments)]
async fn file_task(
    store: &Arc<dyn Store>,
    pool: &SqlitePool,
    n: u128,
    channel: ChannelId,
    planner: MemberId,
    pins: &mut Vec<Pin>,
    title: &str,
    brief: &str,
    filed: i64,
) -> ThreadId {
    let t = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
        })
        .await
        .expect("thread");
    // The thread id is in the review packet's evidence root, which the card
    // prints, so it is fixed too.
    let id = ThreadId(fixed_id(n));
    repin(pool, t.id.0, id.0).await;
    store
        .set_thread_owner(id, Some(planner))
        .await
        .expect("owner");
    let m = store
        .post_message(NewMessage {
            thread_id: id,
            author_id: planner,
            body: brief.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("brief");
    pins.push(Pin {
        key: "id",
        table: "maidan_threads",
        column: "created_at",
        id: id.0,
        at: mins(filed),
    });
    pins.push(Pin {
        key: "id",
        table: "maidan_messages",
        column: "posted_at",
        id: m.id.0,
        at: mins(filed),
    });
    id
}

#[allow(clippy::too_many_lines)]
async fn seed_board(
    store: &Arc<dyn Store>,
    pool: &SqlitePool,
    pins: &mut Vec<Pin>,
    ages: &mut Vec<(uuid::Uuid, i64)>,
    base: u128,
    variant: Variant,
) -> Board {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "maidan".into(),
        })
        .await
        .expect("workspace");
    // The Connect sheet prints the workspace and channel ids.
    let ws_id = WorkspaceId(fixed_id(base));
    repin(pool, ws.id.0, ws_id.0).await;
    let cast = Cast {
        david: member(
            store,
            pool,
            ws_id,
            base + 1,
            "david",
            "David",
            MemberKind::Human,
        )
        .await,
        planner: member(
            store,
            pool,
            ws_id,
            base + 2,
            "planner",
            "Planner",
            MemberKind::Agent,
        )
        .await,
        server: member(
            store,
            pool,
            ws_id,
            base + 3,
            "server-coder",
            "Server coder",
            MemberKind::Agent,
        )
        .await,
        ui: member(
            store,
            pool,
            ws_id,
            base + 4,
            "ui-coder",
            "UI coder",
            MemberKind::Agent,
        )
        .await,
        tester: member(
            store,
            pool,
            ws_id,
            base + 5,
            "test-runner",
            "Test runner",
            MemberKind::Agent,
        )
        .await,
    };
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws_id,
            name: "build".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let channel_id = ChannelId(fixed_id(base + 0xc));
    repin(pool, channel.id.0, channel_id.0).await;
    // What a person pastes into /ui (the quickstart's path). The page trades
    // it for a browser session, and the board's refusal and Live reads use it.
    let david_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws_id,
            member_id: cast.david,
            app_installation_id: None,
            token_hash: hash_secret(david_secret.as_str()),
            label: Some("david".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
                capability::THREAD_TRANSITION.into(),
                capability::EVENT_SUBSCRIBE.into(),
            ],
            expires_at: None,
        })
        .await
        .expect("david token");
    let david_token = david_secret.as_str().to_string();
    if variant == Variant::Empty {
        return Board {
            workspace: ws_id,
            channel: channel_id,
            cast,
            review: None,
            david_token,
            planner_token: None,
        };
    }

    let mut next = base + 0x10;
    macro_rules! file {
        ($title:expr, $brief:expr, $filed:expr) => {{
            next += 1;
            file_task(
                store,
                pool,
                next,
                channel_id,
                cast.planner,
                pins,
                $title,
                $brief,
                $filed,
            )
            .await
        }};
    }
    let say = |thread: ThreadId, who: MemberId, body: &'static str| {
        let store = store.clone();
        async move {
            store
                .post_message(NewMessage {
                    thread_id: thread,
                    author_id: who,
                    body: body.into(),
                    metadata: serde_json::json!({}),
                    content: None,
                })
                .await
                .expect("message")
                .id
                .0
        }
    };
    // Everything the seed wrote about a task (its row, claim, transitions,
    // result, review rows) moves back to `m` minutes before CAPTURE_NOW.
    let touched = |ages: &mut Vec<(uuid::Uuid, i64)>, t: ThreadId, m: i64| ages.push((t.0, m));
    let posted = |pins: &mut Vec<Pin>, id: uuid::Uuid, m: i64| {
        pins.push(Pin {
            key: "id",
            table: "maidan_messages",
            column: "posted_at",
            id,
            at: mins(m),
        });
    };

    // Done: two tasks approved by the test runner and closed.
    for (title, brief, filed, closed) in [
        (
            "Let a workspace set the confirmation link lifetime",
            "Make the ten-minute approval confirmation link a workspace setting: 1 to 60 minutes, ten by default, admins only.",
            420,
            190,
        ),
        (
            "Prove cursor arithmetic cannot overflow",
            "Kani proofs for the cursor helpers. A cursor at i64::MAX must not wrap.",
            480,
            300,
        ),
    ] {
        let t = file!(title, brief, filed);
        let c = store.claim_thread(t, cast.server).await.expect("claim");
        if let Some(lease) = c.thread.claim_lease_id {
            store
                .acknowledge_claim(t, cast.server, lease)
                .await
                .expect("start");
        }
        store
            .set_thread_result(t, cast.server, &serde_json::json!({ "tests": "passed" }))
            .await
            .expect("result");
        store
            .transition_thread(t, cast.server, ThreadAction::StartReview)
            .await
            .expect("review");
        let packet = store
            .latest_review_packet(t)
            .await
            .expect("packet")
            .expect("handed to review");
        store
            .submit_review(
                t,
                cast.tester,
                ReviewDecision::Approve,
                None,
                Some(&packet.evidence_root),
            )
            .await
            .expect("approve");
        store
            .transition_thread(t, cast.planner, ThreadAction::Close)
            .await
            .expect("close");
        touched(ages, t, closed);
    }

    // In review: the server coder's fix, with its test log linked.
    let review = file!(
        "End a deactivated member's WebSocket streams",
        "A stream checks its credential only when it opens. When SCIM deactivates a member, their open streams should close.",
        150
    );
    let c = store
        .claim_thread(review, cast.server)
        .await
        .expect("claim review");
    if let Some(lease) = c.thread.claim_lease_id {
        store
            .acknowledge_claim(review, cast.server, lease)
            .await
            .expect("start review work");
    }
    let m = say(
        review,
        cast.server,
        "The stream now re-checks the token or session and the member's SCIM link before each frame and at every ping, and closes with 1008 and the reason.",
    )
    .await;
    posted(pins, m, 48);
    let m = say(
        review,
        cast.server,
        "Two-tenant test: deactivating a member in one workspace ends only their streams. Reverting the check fails all three tests.",
    )
    .await;
    posted(pins, m, 41);
    // The test log is evidence on the card David decides; the board with
    // work in flight never opens it.
    if variant == Variant::Waiting {
        let log = b"ws_deactivated_member_e2e: 3 passed\nbreak-check (no recheck): 3 failed\n";
        let artifact = store
            .upsert_artifact_with_event(
                NewArtifact {
                    sha256: hex::encode(Sha256::digest(log)),
                    size_bytes: i64::try_from(log.len()).expect("size"),
                    mime_type: Some("text/plain".into()),
                    filename: Some("ws-deactivated-member.log".into()),
                    kind: ArtifactKind::Transcript,
                    uploaded_by: Some(cast.server),
                },
                Some(ws_id),
            )
            .await
            .expect("artifact")
            .0;
        store
            .link_thread_artifact(review, &artifact.sha256, cast.server)
            .await
            .expect("link log");
        pins.push(Pin {
            key: "uploaded_by",
            table: "maidan_artifacts",
            column: "created_at",
            id: cast.server.0,
            at: mins(42),
        });
        pins.push(Pin {
            key: "workspace_id",
            table: "maidan_artifact_refs",
            column: "created_at",
            id: ws_id.0,
            at: mins(42),
        });
    }
    store
        .set_thread_result(
            review,
            cast.server,
            &serde_json::json!({ "tests": "3 passed", "break_checks": "3 of 3 caught" }),
        )
        .await
        .expect("review result");
    store
        .transition_thread(review, cast.server, ThreadAction::StartReview)
        .await
        .expect("start review");
    store
        .set_review_requirement(review, 1)
        .await
        .expect("requirement");
    let reviewer = if variant == Variant::Waiting {
        cast.david
    } else {
        cast.tester
    };
    store
        .add_reviewer(review, reviewer)
        .await
        .expect("reviewer");
    touched(ages, review, 40);

    // In progress: one task each for the server coder and the UI coder.
    for (title, brief, filed, who, note, said, lease_left) in [
        (
            "Re-check credentials on SSE streams",
            "The SSE and long-poll streams have the same gap: a revoked token keeps its stream. Re-check before each frame.",
            95,
            cast.server,
            "Shared recheck is in. Wiring it into /mcp/streamable next.",
            12,
            11,
        ),
        (
            "Keep keyboard focus on Needs you after a refresh",
            "A reload of the Needs you list drops focus from the decision button to the page. Keep it on the same row.",
            70,
            cast.ui,
            "Found it: the render detaches the rows it keeps. Restoring focus by row key.",
            6,
            13,
        ),
    ] {
        let t = file!(title, brief, filed);
        let c = store.claim_thread(t, who).await.expect("claim");
        if let Some(lease) = c.thread.claim_lease_id {
            store.acknowledge_claim(t, who, lease).await.expect("start");
        }
        let m = say(t, who, note).await;
        posted(pins, m, said);
        touched(ages, t, said);
        pins.push(Pin {
            key: "id",
            table: "maidan_threads",
            column: "assignment_expires_at",
            id: t.0,
            at: now() + Duration::minutes(lease_left),
        });
    }

    // Open: filed, nobody holds them yet.
    for (title, brief, filed) in [
        (
            "List the signed-in identity's workspaces",
            "GET /auth/session/workspaces for the workspace switcher, on the session-only routes.",
            30,
        ),
        (
            "Capture the board for the README",
            "A Playwright script that seeds a workspace and captures the console's key screens the same way every run.",
            18,
        ),
    ] {
        let t = file!(title, brief, filed);
        touched(ages, t, filed);
    }

    // The planner's token: it tries to close the task before David has
    // reviewed it, and the server refuses and records why on the thread.
    let planner_token = if variant == Variant::Waiting {
        let secret = TokenSecret::generate();
        store
            .create_api_token(NewApiToken {
                workspace_id: ws_id,
                member_id: cast.planner,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: Some("planner".into()),
                capabilities: vec![
                    capability::WORKSPACE_READ.into(),
                    capability::WORKSPACE_WRITE.into(),
                    capability::MESSAGE_POST.into(),
                    capability::THREAD_TRANSITION.into(),
                ],
                expires_at: None,
            })
            .await
            .expect("planner token");
        Some(secret.as_str().to_string())
    } else {
        None
    };

    Board {
        workspace: ws_id,
        channel: channel_id,
        cast,
        review: Some(review),
        david_token,
        planner_token,
    }
}

/// Rewrite one timestamp text in the format it was stored in.
fn format_like(original: &str, at: DateTime<Utc>) -> String {
    let rfc = at.to_rfc3339_opts(SecondsFormat::AutoSi, original.ends_with('Z'));
    if original.contains('T') {
        rfc
    } else {
        rfc.replacen('T', " ", 1)
    }
}

fn parse_stamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&s.replacen(' ', "T", 1))
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// The `maidan_*` tables and their `*_at` columns, minus `KEEP_WALL_CLOCK`,
/// with whether the table keys rows by `thread_id`.
async fn clock_columns(pool: &SqlitePool) -> Vec<(String, Vec<String>, bool)> {
    let tables: Vec<String> =
        sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'maidan_%'")
            .fetch_all(pool)
            .await
            .expect("tables")
            .iter()
            .map(|r| r.get::<String, _>(0))
            .filter(|t| !KEEP_WALL_CLOCK.iter().any(|k| t.contains(k)))
            .collect();
    let mut out = Vec::new();
    for table in tables {
        let names: Vec<String> = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(pool)
            .await
            .expect("columns")
            .iter()
            .map(|r| r.get::<String, _>("name"))
            .collect();
        let has_thread = names.iter().any(|c| c == "thread_id");
        let cols = names.into_iter().filter(|c| c.ends_with("_at")).collect();
        out.push((table, cols, has_thread));
    }
    out
}

/// Move the `*_at` text of the rows `filter` selects (a SQL condition on one
/// bound uuid, or every row) by `by`.
async fn shift_rows(
    pool: &SqlitePool,
    table: &str,
    col: &str,
    filter: Option<(&str, uuid::Uuid)>,
    by: Duration,
) {
    let cond = filter.map_or(String::new(), |(key, _)| format!(" AND {key} = ?"));
    let sql = format!("SELECT rowid, {col} FROM {table} WHERE typeof({col}) = 'text'{cond}");
    let mut q = sqlx::query(&sql);
    if let Some((_, id)) = filter {
        q = q.bind(id);
    }
    for row in q.fetch_all(pool).await.expect("timestamps") {
        let rowid: i64 = row.get(0);
        let text: String = row.get(1);
        let Some(at) = parse_stamp(&text) else {
            continue;
        };
        sqlx::query(&format!("UPDATE {table} SET {col} = ? WHERE rowid = ?"))
            .bind(format_like(&text, at + by))
            .bind(rowid)
            .execute(pool)
            .await
            .expect("shift timestamp");
    }
}

/// Put every stored timestamp on the capture clock. First every time moves
/// so the end of the seed is `CAPTURE_NOW`; then each task in `ages` moves
/// back by its minutes, with all its rows; then the `pins` are written as
/// given. Tables in `KEEP_WALL_CLOCK` are left alone.
async fn pin_clock(
    pool: &SqlitePool,
    seeded_at: DateTime<Utc>,
    ages: &[(uuid::Uuid, i64)],
    pins: &[Pin],
) {
    // A trigger can refuse the rewrite (a review packet is immutable) or stamp
    // the wall clock back on. The seed is done writing, so every trigger is
    // set aside while the times move, then put back as it was.
    let triggers: Vec<(String, String)> =
        sqlx::query("SELECT name, sql FROM sqlite_master WHERE type = 'trigger'")
            .fetch_all(pool)
            .await
            .expect("triggers")
            .iter()
            .map(|r| (r.get::<String, _>(0), r.get::<String, _>(1)))
            .collect();
    for (name, _) in &triggers {
        sqlx::query(&format!("DROP TRIGGER {name}"))
            .execute(pool)
            .await
            .expect("set trigger aside");
    }
    let columns = clock_columns(pool).await;
    for (table, cols, _) in &columns {
        for col in cols {
            shift_rows(pool, table, col, None, now() - seeded_at).await;
        }
    }
    for (thread, minutes) in ages {
        for (table, cols, has_thread) in &columns {
            let key = if table == "maidan_threads" {
                "id"
            } else if *has_thread {
                "thread_id"
            } else {
                continue;
            };
            for col in cols {
                shift_rows(
                    pool,
                    table,
                    col,
                    Some((key, *thread)),
                    -Duration::minutes(*minutes),
                )
                .await;
            }
        }
    }
    for pin in pins {
        let row = sqlx::query(&format!(
            "SELECT {} FROM {} WHERE {} = ? LIMIT 1",
            pin.column, pin.table, pin.key
        ))
        .bind(pin.id)
        .fetch_one(pool)
        .await
        .expect("pinned row");
        let original: Option<String> = row.get(0);
        let shape = original.unwrap_or_else(|| CAPTURE_NOW.to_string());
        sqlx::query(&format!(
            "UPDATE {} SET {} = ? WHERE {} = ?",
            pin.table, pin.column, pin.key
        ))
        .bind(format_like(&shape, pin.at))
        .bind(pin.id)
        .execute(pool)
        .await
        .expect("write pin");
    }
    for (_, sql) in &triggers {
        sqlx::query(sql)
            .execute(pool)
            .await
            .expect("put trigger back");
    }
}

/// Print every timestamp within two minutes of `CAPTURE_NOW`: what the seed
/// did not pin reads as "just now" on the page.
async fn debug_recent(pool: &SqlitePool) {
    let tables: Vec<String> =
        sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'maidan_%'")
            .fetch_all(pool)
            .await
            .expect("tables")
            .iter()
            .map(|r| r.get::<String, _>(0))
            .collect();
    for table in tables {
        let cols: Vec<String> = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(pool)
            .await
            .expect("columns")
            .iter()
            .map(|r| r.get::<String, _>("name"))
            .filter(|c| c.ends_with("_at"))
            .collect();
        for col in cols {
            let rows = sqlx::query(&format!(
                "SELECT {col} FROM {table} WHERE typeof({col}) = 'text'"
            ))
            .fetch_all(pool)
            .await
            .expect("timestamps");
            for row in rows {
                let text: String = row.get(0);
                if let Some(at) = parse_stamp(&text) {
                    if (at - now()).num_seconds().abs() < 120 {
                        eprintln!("capture_server: recent {table}.{col} = {text}");
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("CAPTURE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8961);
    let fixtures_path =
        std::env::var("CAPTURE_FIXTURES").unwrap_or_else(|_| "ui-tests/.capture.json".into());

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));

    let mut pins = Vec::new();
    let mut ages = Vec::new();
    let flight = seed_board(&store, &pool, &mut pins, &mut ages, 0x100, Variant::Flight).await;
    let waiting = seed_board(&store, &pool, &mut pins, &mut ages, 0x200, Variant::Waiting).await;
    let empty = seed_board(&store, &pool, &mut pins, &mut ages, 0x300, Variant::Empty).await;

    let art_dir = std::env::temp_dir().join(format!("maidan-capture-{}", std::process::id()));
    std::fs::create_dir_all(&art_dir).expect("art dir");
    let mut state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(&art_dir)),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth enabled: the page signs in with a real session cookie
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(&b"capture-subscribe-resume-secret-32b"[..]));
    state.console_origin = Some(format!("http://127.0.0.1:{port}"));
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: Arc::from(SESSION_SECRET),
        ttl_secs: 28_800,
        cookie_secure: false,
    });
    let app = router(state);

    // The planner closes the task while it still waits on David's review.
    // The real route refuses it and posts the refusal on the thread, which
    // the card draws as "Close refused".
    if let (Some(token), Some(review)) = (&waiting.planner_token, waiting.review) {
        let req = axum::http::Request::post(format!("/threads/{}", review.0))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"action":"close"}"#))
            .expect("request");
        let mut svc = app.clone();
        let res = tower::Service::call(&mut svc, req)
            .await
            .expect("refused close");
        assert_eq!(
            res.status(),
            axum::http::StatusCode::CONFLICT,
            "a close before the review must be refused"
        );
        let refusal = sqlx::query(
            "SELECT id FROM maidan_messages WHERE thread_id = ? ORDER BY rowid DESC LIMIT 1",
        )
        .bind(review.0)
        .fetch_one(&pool)
        .await
        .expect("refusal message");
        pins.push(Pin {
            key: "id",
            table: "maidan_messages",
            column: "posted_at",
            id: refusal.get::<uuid::Uuid, _>(0),
            at: mins(39),
        });
    }

    pin_clock(&pool, Utc::now(), &ages, &pins).await;
    if std::env::var_os("CAPTURE_DEBUG").is_some() {
        debug_recent(&pool).await;
    }

    let board = |b: &Board| {
        serde_json::json!({
            "workspace_id": b.workspace.0.to_string(),
            "channel_id": b.channel.0.to_string(),
            "token": b.david_token,
            "david_id": b.cast.david.0.to_string(),
            "review_thread_id": b.review.map(|t| t.0.to_string()),
        })
    };
    let fixtures = serde_json::json!({
        "base_url": format!("http://127.0.0.1:{port}"),
        "now": CAPTURE_NOW,
        "flight": board(&flight),
        "waiting": board(&waiting),
        "empty": board(&empty),
    });
    std::fs::write(
        &fixtures_path,
        serde_json::to_string_pretty(&fixtures).expect("fixtures json"),
    )
    .expect("write fixtures");

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind (is CAPTURE_PORT free?)");
    eprintln!("capture_server: seeded + listening on http://{addr} (fixtures: {fixtures_path})");
    axum::serve(listener, app).await.expect("serve");
}
