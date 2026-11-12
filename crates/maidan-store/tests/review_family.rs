//! Second-family review routing on both backends (S4): a change entering
//! review spawns a review child routed away from its author's provider
//! family, and `claim_next` refuses that child to a seat whose usage
//! evidence names the author's family. A review seat that claims by
//! `kind = review` and its repository, with no engine, is handed every review
//! there whatever engine wrote the change, and never a change.

use maidan_store::test_support::SetResultClaiming;
use std::time::Duration;

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

/// One usage record: `reporter` used `model` on `thread`.
struct Usage {
    ws: Uuid,
    thread: Uuid,
    reporter: Uuid,
    model: &'static str,
}

/// The scenario, with `record` writing a usage record on the backend.
async fn run_suite<F, Fut>(store: &dyn Store, record: F)
where
    F: Fn(Usage) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let ws = store
        .create_workspace(NewWorkspace {
            name: "families".into(),
        })
        .await
        .expect("workspace")
        .id;
    let mut ids = Vec::new();
    for handle in ["author", "same", "other", "roamer"] {
        ids.push(
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
    let (author, same, other, roamer) = (ids[0], ids[1], ids[2], ids[3]);
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let new = |title: &str| NewThread {
        channel_id: channel,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: None,
    };

    // A change with no usage evidence spawns no review child.
    let unrouted = store
        .create_thread(new("unrouted"))
        .await
        .expect("thread")
        .id;
    store.claim_thread(unrouted, author).await.expect("claim");
    store
        .set_thread_result_claiming(unrouted, author, &json!({"status": "done"}))
        .await
        .expect("result");
    store
        .transition_thread(unrouted, author, ThreadAction::StartReview)
        .await
        .expect("start review");
    assert!(
        store
            .claim_next_thread(channel, other, None, None)
            .await
            .expect("claim next")
            .is_none(),
        "no family named, nothing routed"
    );

    // A second tenant, where `roamer` once reported under the author's
    // family: evidence filed in another workspace refuses nothing here.
    let elsewhere = store
        .create_workspace(NewWorkspace {
            name: "elsewhere".into(),
        })
        .await
        .expect("workspace")
        .id;
    let elsewhere_channel = store
        .create_channel(NewChannel {
            workspace_id: elsewhere,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let elsewhere_thread = store
        .create_thread(NewThread {
            channel_id: elsewhere_channel,
            parent_thread_id: None,
            title: Some("elsewhere".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id;
    record(Usage {
        ws: elsewhere.0,
        thread: elsewhere_thread.0,
        reporter: roamer.0,
        model: "claude-opus-4",
    })
    .await;

    // A change written by an Anthropic seat. Other seats report on it too,
    // a different family last: only the claimant's own records name the
    // author's family.
    let change = store.create_thread(new("change")).await.expect("thread").id;
    store.claim_thread(change, author).await.expect("claim");
    for (reporter, model) in [
        (author, "claude-sonnet-4"),
        (same, "claude-opus-4"),
        (other, "gpt-5-codex"),
        (other, "o4-mini"),
    ] {
        record(Usage {
            ws: ws.0,
            thread: change.0,
            reporter: reporter.0,
            model,
        })
        .await;
    }
    store
        .set_thread_result_claiming(change, author, &json!({"status": "done"}))
        .await
        .expect("result");
    let (_, events) = store
        .transition_thread_with_event(change, author, ThreadAction::StartReview)
        .await
        .expect("start review");
    // The spawned review child's routing rides out with the transition so a
    // live subscriber sees it (#1572).
    let routed = events
        .iter()
        .map(|e| e.opened_event().expect("event"))
        .find_map(|e| match e {
            Event::ThreadAttributesSet {
                attributes,
                author_family,
                ..
            } => Some((attributes, author_family)),
            _ => None,
        })
        .expect("the review child's routing is published");
    assert_eq!(routed.0.kind, TaskKind::Review);
    assert_eq!(routed.0.review_family.as_deref(), Some("openai"));
    assert_eq!(routed.1.as_deref(), Some("anthropic"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.opened_event(), Ok(Event::ThreadCreated { .. }))),
        "the review child's creation is published"
    );

    for seat in [author, same] {
        assert!(
            store
                .claim_next_thread(channel, seat, None, None)
                .await
                .expect("claim next")
                .is_none(),
            "a seat of the author's family never gets the review"
        );
    }
    let review = store
        .claim_next_thread(channel, roamer, None, None)
        .await
        .expect("claim next")
        .expect("another tenant's evidence does not refuse the review");
    assert_eq!(review.parent_thread_id, Some(change));
    let attributes = store.get_thread_attributes(review.id).await.expect("attrs");
    assert_eq!(attributes.kind, TaskKind::Review);
    assert_eq!(attributes.review_family.as_deref(), Some("openai"));

    // Setting attributes leaves the server-set routing as it was.
    let restated = store
        .set_thread_attributes(
            review.id,
            TaskAttributes {
                kind: TaskKind::Review,
                ..TaskAttributes::default()
            },
        )
        .await
        .expect("set");
    assert_eq!(restated.review_family.as_deref(), Some("openai"));

    // With review children off, a change entering review spawns none, and
    // the other tenant keeps the default.
    assert_eq!(
        store.workspace_review_children(ws).await.expect("read"),
        ReviewChildren::Spawn
    );
    store
        .set_workspace_review_children(
            ws,
            ReviewChildren::Off,
            NewAuditEvent {
                scope: AuditScope::Workspace(ws),
                actor_id: None,
                action: "workspace.review_children.set".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(ws.0),
                metadata: json!({ "review_children": "off" }),
            },
        )
        .await
        .expect("set off");
    assert_eq!(
        store.workspace_review_children(ws).await.expect("read"),
        ReviewChildren::Off
    );
    assert_eq!(
        store
            .workspace_review_children(elsewhere)
            .await
            .expect("read"),
        ReviewChildren::Spawn,
        "one workspace's setting never reaches another"
    );
    let quiet = store.create_thread(new("quiet")).await.expect("thread").id;
    store.claim_thread(quiet, author).await.expect("claim");
    record(Usage {
        ws: ws.0,
        thread: quiet.0,
        reporter: author.0,
        model: "claude-sonnet-4",
    })
    .await;
    store
        .set_thread_result_claiming(quiet, author, &json!({"status": "done"}))
        .await
        .expect("result");
    store
        .transition_thread(quiet, author, ThreadAction::StartReview)
        .await
        .expect("start review");
    for seat in [other, roamer] {
        assert!(
            store
                .claim_next_thread(channel, seat, None, None)
                .await
                .expect("claim next")
                .is_none(),
            "with review children off no review task is spawned"
        );
    }

    // A review a producer files for a change written elsewhere names the
    // author's family itself, and claim_next refuses it to that family.
    let (filed, _) = store
        .create_thread_with_footprint(
            new("filed review"),
            None,
            Some(TaskAttributes {
                kind: TaskKind::Review,
                author_family: Some("openai".into()),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .expect("filed review");
    assert!(
        store
            .claim_next_thread(channel, other, None, None)
            .await
            .expect("claim next")
            .is_none(),
        "an openai seat never gets a review of an openai change"
    );
    // Whoever may set attributes cannot lift it: a set that omits the
    // family keeps it, and one that names another is refused.
    let kept = store
        .set_thread_attributes(
            filed.id,
            TaskAttributes {
                kind: TaskKind::Review,
                ..TaskAttributes::default()
            },
        )
        .await
        .expect("set");
    assert_eq!(kept.author_family.as_deref(), Some("openai"));
    let renamed = store
        .set_thread_attributes(
            filed.id,
            TaskAttributes {
                kind: TaskKind::Review,
                author_family: Some("anthropic".into()),
                ..TaskAttributes::default()
            },
        )
        .await;
    assert!(
        matches!(&renamed, Err(StoreError::Conflict(m)) if m.contains("stays openai")),
        "{renamed:?}"
    );
    assert_eq!(
        store
            .get_thread_attributes(filed.id)
            .await
            .expect("attrs")
            .author_family
            .as_deref(),
        Some("openai")
    );
    // A direct claim or an assign by a seat of the author's family is
    // refused too, naming the family, not answered with a quiet miss.
    let direct = store.claim_thread(filed.id, other).await;
    assert!(
        matches!(&direct, Err(StoreError::Conflict(m)) if m.contains("openai family")),
        "{direct:?}"
    );
    let assigned = store.assign_thread(filed.id, other).await;
    assert!(
        matches!(&assigned, Err(StoreError::Conflict(m)) if m.contains("openai family")),
        "{assigned:?}"
    );
    let direct = store.claim_thread_with_event(filed.id, other).await;
    assert!(
        matches!(&direct, Err(StoreError::Conflict(m)) if m.contains("openai family")),
        "{direct:?}"
    );
    let assigned = store
        .assign_thread_with_event(filed.id, other, author, None)
        .await;
    assert!(
        matches!(&assigned, Err(StoreError::Conflict(m)) if m.contains("openai family")),
        "{assigned:?}"
    );
    assert!(
        store
            .claim_next_thread(channel, other, None, None)
            .await
            .expect("claim next")
            .is_none(),
        "still refused after the sets"
    );
    let taken = store
        .claim_next_thread(channel, same, None, None)
        .await
        .expect("claim next")
        .expect("an anthropic seat takes it");
    assert_eq!(taken.id, filed.id);
    // A member of another family may still be handed it directly.
    let handed = store
        .assign_thread(filed.id, author)
        .await
        .expect("assign to an anthropic member");
    assert_eq!(handed.assignee_id, Some(author));

    // A set may name the family of a review filed without one.
    let (unnamed, _) = store
        .create_thread_with_footprint(
            new("unnamed review"),
            None,
            Some(TaskAttributes {
                kind: TaskKind::Review,
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .expect("unnamed review");
    let named = store
        .set_thread_attributes(
            unnamed.id,
            TaskAttributes {
                kind: TaskKind::Review,
                author_family: Some("anthropic".into()),
                ..TaskAttributes::default()
            },
        )
        .await
        .expect("set");
    assert_eq!(named.author_family.as_deref(), Some("anthropic"));
    for seat in [author, same] {
        assert!(
            store
                .claim_next_thread(channel, seat, None, None)
                .await
                .expect("claim next")
                .is_none(),
            "an anthropic seat never gets it once named"
        );
    }
    let direct = store
        .claim_thread(unnamed.id, other)
        .await
        .expect("an openai member claims an anthropic change's review");
    assert!(direct.claimed);

    // Naming the family after a seat of that family already holds the
    // review is refused and changes nothing: every claim-time check ran
    // while the thread was unlabeled, so the set is the last place to stop it.
    let (late, _) = store
        .create_thread_with_footprint(
            new("review labelled late"),
            None,
            Some(TaskAttributes {
                kind: TaskKind::Review,
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .expect("late review");
    assert!(
        store
            .claim_thread(late.id, other)
            .await
            .expect("claim")
            .claimed
    );
    let labelled = store
        .set_thread_attributes(
            late.id,
            TaskAttributes {
                kind: TaskKind::Review,
                author_family: Some("openai".into()),
                ..TaskAttributes::default()
            },
        )
        .await;
    assert!(
        matches!(&labelled, Err(StoreError::Conflict(m)) if m.contains("openai family")),
        "{labelled:?}"
    );
    assert_eq!(
        store
            .get_thread_attributes(late.id)
            .await
            .expect("attrs")
            .author_family,
        None,
        "the refused set stored nothing"
    );

    // A claim racing the label: whichever commits second sees the first, so
    // an openai seat never ends up holding an openai-labelled review.
    for round in 0..100 {
        let (race, _) = store
            .create_thread_with_footprint(
                new(&format!("review raced {round}")),
                None,
                Some(TaskAttributes {
                    kind: TaskKind::Review,
                    ..TaskAttributes::default()
                }),
                None,
            )
            .await
            .expect("raced review");
        let (claim, label) = tokio::join!(
            store.claim_thread(race.id, other),
            store.set_thread_attributes(
                race.id,
                TaskAttributes {
                    kind: TaskKind::Review,
                    author_family: Some("openai".into()),
                    ..TaskAttributes::default()
                },
            )
        );
        let claimed = matches!(&claim, Ok(c) if c.claimed);
        assert!(
            !(claimed && label.is_ok()),
            "round {round}: both the claim and the label committed"
        );
        let held = store.get_thread(race.id).await.expect("thread").assignee_id;
        let family = store
            .get_thread_attributes(race.id)
            .await
            .expect("attrs")
            .author_family;
        assert!(
            !(held == Some(other) && family.as_deref() == Some("openai")),
            "round {round}: an openai seat holds an openai review"
        );
    }
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
async fn a_review_is_routed_away_from_its_authors_family_on_sqlite() {
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
    run_suite(&store, |u| {
        let pool = pool.clone();
        async move {
            sqlx::query(INSERT_SQLITE)
                .bind(Uuid::now_v7())
                .bind(u.ws)
                .bind(u.thread)
                .bind(u.reporter)
                .bind(Uuid::now_v7())
                .bind(u.model)
                .execute(&pool)
                .await
                .expect("usage");
        }
    })
    .await;
}

#[tokio::test]
async fn a_review_is_routed_away_from_its_authors_family_on_postgres() {
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
    run_suite(&store, |u| {
        let pool = pool.clone();
        async move {
            sqlx::query(INSERT_POSTGRES)
                .bind(Uuid::now_v7())
                .bind(u.ws)
                .bind(u.thread)
                .bind(u.reporter)
                .bind(Uuid::now_v7())
                .bind(u.model)
                .execute(&pool)
                .await
                .expect("usage");
        }
    })
    .await;
}

const SQLITE_0230: &str =
    include_str!("../../../migrations/sqlite/0230_review_served_where_its_change_is.sql");
const POSTGRES_0230: &str =
    include_str!("../../../migrations/postgres/0230_review_served_where_its_change_is.sql");

/// The backend under a store, for the writes the store has no call for: a
/// usage record, and a migration run again.
#[derive(Clone)]
enum Pool {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
}

impl Pool {
    async fn usage(&self, u: Usage) {
        let done = match self {
            Self::Sqlite(pool) => sqlx::query(INSERT_SQLITE)
                .bind(Uuid::now_v7())
                .bind(u.ws)
                .bind(u.thread)
                .bind(u.reporter)
                .bind(Uuid::now_v7())
                .bind(u.model)
                .execute(pool)
                .await
                .map(|_| ()),
            Self::Postgres(pool) => sqlx::query(INSERT_POSTGRES)
                .bind(Uuid::now_v7())
                .bind(u.ws)
                .bind(u.thread)
                .bind(u.reporter)
                .bind(Uuid::now_v7())
                .bind(u.model)
                .execute(pool)
                .await
                .map(|_| ()),
        };
        done.expect("usage");
    }

    /// Run migration 0230 again on rows written after it first ran.
    async fn rerun_0230(&self) {
        let done = match self {
            Self::Sqlite(pool) => sqlx::raw_sql(SQLITE_0230).execute(pool).await.map(|_| ()),
            Self::Postgres(pool) => sqlx::raw_sql(POSTGRES_0230).execute(pool).await.map(|_| ()),
        };
        done.expect("re-run 0230");
    }

    async fn close(&self, thread: ThreadId) {
        let done = match self {
            Self::Sqlite(pool) => {
                sqlx::query("UPDATE maidan_threads SET state = 'closed' WHERE id = ?")
                    .bind(thread.0)
                    .execute(pool)
                    .await
                    .map(|_| ())
            }
            Self::Postgres(pool) => {
                sqlx::query("UPDATE maidan_threads SET state = 'closed' WHERE id = $1")
                    .bind(thread.0)
                    .execute(pool)
                    .await
                    .map(|_| ())
            }
        };
        done.expect("close");
    }
}

const REPO: &str = "example/widgets";

/// A workspace with an Anthropic seat that writes the changes and a DeepSeek
/// seat of a third family, whose family is on record there.
struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    author: MemberId,
    third: MemberId,
}

async fn room(store: &dyn Store, pool: &Pool, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace")
        .id;
    let mut ids = Vec::new();
    for handle in ["author", "third"] {
        ids.push(
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
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let (author, third) = (ids[0], ids[1]);
    // The DeepSeek seat's family is on record before any review exists.
    let elsewhere = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("deepseek's earlier work".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id;
    pool.usage(Usage {
        ws: ws.0,
        thread: elsewhere.0,
        reporter: third.0,
        model: "deepseek-chat",
    })
    .await;
    pool.close(elsewhere).await;
    Room {
        ws,
        channel,
        author,
        third,
    }
}

/// What `claim_next` filtered by `repo` and `engine` hands `member` in
/// `room`'s channel.
async fn filtered_claim(
    store: &dyn Store,
    room: &Room,
    member: MemberId,
    repo: &str,
    engine: TaskEngine,
) -> Option<Thread> {
    store
        .claim_next_thread_with_event(
            room.channel,
            member,
            None,
            Some(repo),
            Some(engine),
            None,
            None,
        )
        .await
        .expect("claim next")
        .0
}

/// A review child of `change` as the spawn wrote one before it copied
/// anything: the kind and the author's family only.
async fn old_review(store: &dyn Store, room: &Room, change: ThreadId, title: &str) -> ThreadId {
    store
        .create_thread_with_footprint(
            NewThread {
                channel_id: room.channel,
                parent_thread_id: Some(change),
                title: Some(title.into()),
                description: None,
                created_by: None,
            },
            None,
            Some(TaskAttributes {
                kind: TaskKind::Review,
                author_family: Some("anthropic".into()),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .expect("review")
        .0
        .id
}

fn exclusive(path: &str) -> Footprint {
    Footprint {
        repo: REPO.into(),
        mode: FootprintMode::Exclusive,
        items: vec![FootprintItem {
            kind: FootprintKind::Path,
            value: path.into(),
        }],
    }
}

/// A change in `room` with `footprint` and `attributes`, written by the
/// author and sent to review; returns the change and the review it spawned.
async fn change_into_review(
    store: &dyn Store,
    pool: &Pool,
    room: &Room,
    title: &str,
    footprint: Option<Footprint>,
    attributes: TaskAttributes,
) -> (ThreadId, ThreadId) {
    let (change, _) = store
        .create_thread_with_footprint(
            NewThread {
                channel_id: room.channel,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
                created_by: None,
            },
            footprint,
            Some(attributes),
            None,
        )
        .await
        .expect("change");
    store
        .claim_thread(change.id, room.author)
        .await
        .expect("claim");
    pool.usage(Usage {
        ws: room.ws.0,
        thread: change.id.0,
        reporter: room.author.0,
        model: "claude-sonnet-4",
    })
    .await;
    store
        .set_thread_result_claiming(change.id, room.author, &json!({"status": "done"}))
        .await
        .expect("result");
    let (_, events) = store
        .transition_thread_with_event(change.id, room.author, ThreadAction::StartReview)
        .await
        .expect("start review");
    let review = events
        .iter()
        .map(|e| e.opened_event().expect("event"))
        .find_map(|e| match e {
            Event::ThreadCreated { thread, .. } if thread.parent_thread_id == Some(change.id) => {
                Some(thread.id)
            }
            _ => None,
        })
        .expect("a review child is spawned");
    (change.id, review)
}

/// A spawned review carries its change's engine, repository and lane, and
/// its author's family; it holds no footprint, so it waits on no claim and
/// no claim waits on it. A change that names its repository only in its
/// footprint lends the review that one.
async fn a_spawned_review_carries_its_changes_engine_repository_and_lane(
    store: &dyn Store,
    pool: &Pool,
) {
    let room = room(store, pool, "carries").await;
    let (_, review) = change_into_review(
        store,
        pool,
        &room,
        "named change",
        Some(exclusive("src/")),
        TaskAttributes {
            kind: TaskKind::Change,
            engine: Some(TaskEngine::Session),
            repo: Some(REPO.into()),
            lane: Some(TaskLane::Flex),
            ..TaskAttributes::default()
        },
    )
    .await;
    let attributes = store.get_thread_attributes(review).await.expect("attrs");
    assert_eq!(attributes.kind, TaskKind::Review);
    assert_eq!(attributes.engine, Some(TaskEngine::Session));
    assert_eq!(attributes.repo.as_deref(), Some(REPO));
    assert_eq!(attributes.lane, Some(TaskLane::Flex));
    assert_eq!(attributes.author_family.as_deref(), Some("anthropic"));
    assert_eq!(
        store.get_thread_footprint(review).await.expect("footprint"),
        None,
        "a review holds nothing"
    );

    let (_, review) = change_into_review(
        store,
        pool,
        &room,
        "footprint-only change",
        Some(exclusive("docs/")),
        TaskAttributes {
            kind: TaskKind::Change,
            engine: Some(TaskEngine::Session),
            ..TaskAttributes::default()
        },
    )
    .await;
    let attributes = store.get_thread_attributes(review).await.expect("attrs");
    assert_eq!(attributes.engine, Some(TaskEngine::Session));
    assert_eq!(
        attributes.repo.as_deref(),
        Some(REPO),
        "the change's footprint names the repository"
    );
    assert_eq!(attributes.lane, None);
}

/// `claim_next` filtered by the change's engine and repository hands its
/// review to a seat of a third family, never to the author's; a seat that
/// serves another engine or repository is not handed it. Only a review
/// matches by its `repo` attribute: a change still needs a footprint.
async fn a_third_family_seat_filtered_by_engine_and_repository_claims_the_review(
    store: &dyn Store,
    pool: &Pool,
) {
    let room = room(store, pool, "filtered").await;
    let (_, review) = change_into_review(
        store,
        pool,
        &room,
        "change",
        Some(exclusive("src/")),
        TaskAttributes {
            kind: TaskKind::Change,
            engine: Some(TaskEngine::Session),
            repo: Some(REPO.into()),
            ..TaskAttributes::default()
        },
    )
    .await;
    assert!(
        filtered_claim(store, &room, room.author, REPO, TaskEngine::Session)
            .await
            .is_none(),
        "the author's family is refused"
    );
    let direct = store.claim_thread(review, room.author).await;
    assert!(
        matches!(&direct, Err(StoreError::Conflict(m)) if m.contains("anthropic family")),
        "{direct:?}"
    );
    assert!(
        filtered_claim(
            store,
            &room,
            room.third,
            "example/other",
            TaskEngine::Session
        )
        .await
        .is_none(),
        "another repository's seat"
    );
    assert!(
        filtered_claim(store, &room, room.third, REPO, TaskEngine::Unattended)
            .await
            .is_none(),
        "another engine's seat"
    );
    let handed = filtered_claim(store, &room, room.third, REPO, TaskEngine::Session)
        .await
        .expect("the third family's seat is handed the review");
    assert_eq!(handed.id, review);

    // A change that names the repository only in its attributes has not
    // declared what it touches, so no repository seat is handed it; a seat
    // serving any repository is.
    let (bare, _) = store
        .create_thread_with_footprint(
            NewThread {
                channel_id: room.channel,
                parent_thread_id: None,
                title: Some("change without a footprint".into()),
                description: None,
                created_by: None,
            },
            None,
            Some(TaskAttributes {
                kind: TaskKind::Change,
                engine: Some(TaskEngine::Session),
                repo: Some(REPO.into()),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .expect("bare change");
    assert!(
        filtered_claim(store, &room, room.third, REPO, TaskEngine::Session)
            .await
            .is_none(),
        "a change with no footprint is not handed to a repository seat"
    );
    let unfiltered = store
        .claim_next_thread_with_event(
            room.channel,
            room.author,
            None,
            None,
            Some(TaskEngine::Session),
            None,
            None,
        )
        .await
        .expect("claim next")
        .0
        .expect("a seat serving any repository takes it");
    assert_eq!(unfiltered.id, bare.id);
}

/// Migration 0230, run again over a review spawned before the spawn copied
/// anything: the waiting review takes its change's engine and repository and
/// is handed out by a filtered `claim_next`; a closed review is untouched, and
/// a second run changes nothing.
async fn the_0230_backfill_serves_a_waiting_review_and_leaves_a_closed_one(
    store: &dyn Store,
    pool: &Pool,
) {
    let room = room(store, pool, "backfill").await;
    // A change with no usage on record spawns no review of its own, so the
    // reviews below stand as the old spawn left them.
    let (change, _) = store
        .create_thread_with_footprint(
            NewThread {
                channel_id: room.channel,
                parent_thread_id: None,
                title: Some("change".into()),
                description: None,
                created_by: None,
            },
            Some(exclusive("src/")),
            Some(TaskAttributes {
                kind: TaskKind::Change,
                engine: Some(TaskEngine::Session),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .expect("change");
    store
        .claim_thread(change.id, room.author)
        .await
        .expect("claim");
    store
        .set_thread_result_claiming(change.id, room.author, &json!({"status": "done"}))
        .await
        .expect("result");
    store
        .transition_thread(change.id, room.author, ThreadAction::StartReview)
        .await
        .expect("start review");
    let waiting = old_review(store, &room, change.id, "waiting review").await;
    let closed = old_review(store, &room, change.id, "closed review").await;
    pool.close(closed).await;
    assert!(
        filtered_claim(store, &room, room.third, REPO, TaskEngine::Session)
            .await
            .is_none(),
        "before the backfill no filtered seat sees it"
    );

    pool.rerun_0230().await;
    pool.rerun_0230().await;

    let attributes = store.get_thread_attributes(waiting).await.expect("attrs");
    assert_eq!(attributes.engine, Some(TaskEngine::Session));
    assert_eq!(
        attributes.repo.as_deref(),
        Some(REPO),
        "the change's footprint names it"
    );
    assert_eq!(attributes.author_family.as_deref(), Some("anthropic"));
    let untouched = store.get_thread_attributes(closed).await.expect("attrs");
    assert_eq!(untouched.engine, None);
    assert_eq!(untouched.repo, None);
    assert_eq!(
        store
            .get_thread_footprint(waiting)
            .await
            .expect("footprint"),
        None
    );
    let handed = filtered_claim(store, &room, room.third, REPO, TaskEngine::Session)
        .await
        .expect("the backfilled review is handed out");
    assert_eq!(handed.id, waiting);
}

/// A change the flow delivered names, on its review, the pull request and
/// the head to review, so a reviewer reads both from the thread; one it
/// never delivered keeps the plain title.
async fn a_spawned_review_names_the_pull_request_and_head_it_reviews(
    store: &dyn Store,
    pool: &Pool,
) {
    let room = room(store, pool, "names").await;
    let (change, _) = store
        .create_thread_with_footprint(
            NewThread {
                channel_id: room.channel,
                parent_thread_id: None,
                title: Some("delivered change".into()),
                description: None,
                created_by: None,
            },
            Some(exclusive("src/")),
            Some(TaskAttributes {
                kind: TaskKind::Change,
                engine: Some(TaskEngine::Session),
                repo: Some(REPO.into()),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .expect("change");
    store
        .claim_thread(change.id, room.author)
        .await
        .expect("claim");
    pool.usage(Usage {
        ws: room.ws.0,
        thread: change.id.0,
        reporter: room.author.0,
        model: "claude-sonnet-4",
    })
    .await;
    let head = "0123456789abcdef0123456789abcdef01234567";
    store
        .record_change_round(NewChangeRound {
            workspace_id: room.ws,
            thread_id: change.id,
            repo: REPO.into(),
            pull_number: 42,
            commit_sha: head.into(),
            base_sha: "b".repeat(40),
            base_behind: None,
            diff_lines: 3,
            paths: vec!["src/lib.rs".into()],
            base_branch: Some("dev".into()),
            rebase: false,
            previous_head: None,
            linear_issue: None,
        })
        .await
        .expect("round");
    store
        .set_thread_result_claiming(change.id, room.author, &json!({"status": "done"}))
        .await
        .expect("result");
    let (_, events) = store
        .transition_thread_with_event(change.id, room.author, ThreadAction::StartReview)
        .await
        .expect("start review");
    let review = events
        .iter()
        .map(|e| e.opened_event().expect("event"))
        .find_map(|e| match e {
            Event::ThreadCreated { thread, .. } if thread.parent_thread_id == Some(change.id) => {
                Some(thread)
            }
            _ => None,
        })
        .expect("a review child is spawned");
    assert_eq!(
        review.title.as_deref(),
        Some(format!("Second-family review: {REPO}#42 at 0123456").as_str())
    );
    assert_eq!(
        review.description.as_deref(),
        Some(format!("Pull request: {REPO}#42\nHead to review: {head}").as_str())
    );

    let (_, plain) = change_into_review(
        store,
        pool,
        &room,
        "undelivered change",
        Some(exclusive("docs/")),
        TaskAttributes {
            kind: TaskKind::Change,
            engine: Some(TaskEngine::Session),
            repo: Some(REPO.into()),
            ..TaskAttributes::default()
        },
    )
    .await;
    let plain = store.get_thread(plain).await.expect("review");
    assert_eq!(plain.title.as_deref(), Some("Second-family review"));
    assert_eq!(plain.description, None);
}

async fn served_where_the_change_is(store: &dyn Store, pool: &Pool) {
    a_spawned_review_names_the_pull_request_and_head_it_reviews(store, pool).await;
    a_spawned_review_carries_its_changes_engine_repository_and_lane(store, pool).await;
    a_third_family_seat_filtered_by_engine_and_repository_claims_the_review(store, pool).await;
    the_0230_backfill_serves_a_waiting_review_and_leaves_a_closed_one(store, pool).await;
}

#[tokio::test]
async fn a_review_is_served_where_its_change_is_on_sqlite() {
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
    served_where_the_change_is(&store, &Pool::Sqlite(pool)).await;
}

#[tokio::test]
async fn a_review_is_served_where_its_change_is_on_postgres() {
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
    served_where_the_change_is(&store, &Pool::Postgres(pool)).await;
}

/// What `claim_next` filtered by `kind` and `repo`, with no engine, hands
/// `member` in `room`'s channel.
async fn kind_claim(
    store: &dyn Store,
    room: &Room,
    member: MemberId,
    kind: TaskKind,
    repo: Option<&str>,
) -> Option<Thread> {
    store
        .claim_next_thread_with_event(room.channel, member, None, repo, None, None, Some(kind))
        .await
        .expect("claim next")
        .0
}

/// An open change in `room` that declares `REPO` in its footprint and
/// attributes, routed to `engine`: work a repository seat could take.
async fn open_change(store: &dyn Store, room: &Room, title: &str, engine: TaskEngine) -> ThreadId {
    store
        .create_thread_with_footprint(
            NewThread {
                channel_id: room.channel,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
                created_by: None,
            },
            Some(exclusive("lib/")),
            Some(TaskAttributes {
                kind: TaskKind::Change,
                engine: Some(engine),
                repo: Some(REPO.into()),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .expect("open change")
        .0
        .id
}

/// A review seat that claims with `kind = review` and the repository, and no
/// engine, is handed the review of an unattended-engine change and of a
/// session-engine change, and never an open change in that repository; the
/// author's family is still refused both reviews. `kind = change` with the
/// same repository is handed the change.
async fn a_review_seat_claims_every_review_in_its_repository_whatever_the_engine(
    store: &dyn Store,
    pool: &Pool,
) {
    let room = room(store, pool, "kind-claim").await;
    let (_, unattended_review) = change_into_review(
        store,
        pool,
        &room,
        "unattended change",
        Some(exclusive("src/")),
        TaskAttributes {
            kind: TaskKind::Change,
            engine: Some(TaskEngine::Unattended),
            repo: Some(REPO.into()),
            ..TaskAttributes::default()
        },
    )
    .await;
    let (_, session_review) = change_into_review(
        store,
        pool,
        &room,
        "session change",
        Some(exclusive("docs/")),
        TaskAttributes {
            kind: TaskKind::Change,
            engine: Some(TaskEngine::Session),
            repo: Some(REPO.into()),
            ..TaskAttributes::default()
        },
    )
    .await;
    let change = open_change(store, &room, "open change", TaskEngine::Session).await;

    assert!(
        kind_claim(store, &room, room.author, TaskKind::Review, Some(REPO))
            .await
            .is_none(),
        "the author's family is refused both reviews, and the change is not a review"
    );
    let mut handed = Vec::new();
    for _ in 0..2 {
        let review = kind_claim(store, &room, room.third, TaskKind::Review, Some(REPO))
            .await
            .expect("the review seat is handed a review");
        handed.push(review.id);
    }
    handed.sort_by_key(|id| id.0);
    let mut expected = vec![unattended_review, session_review];
    expected.sort_by_key(|id| id.0);
    assert_eq!(handed, expected, "both engines' reviews, one each");
    assert!(
        kind_claim(store, &room, room.third, TaskKind::Review, Some(REPO))
            .await
            .is_none(),
        "no change thread is ever handed to a review seat"
    );
    assert!(
        kind_claim(
            store,
            &room,
            room.third,
            TaskKind::Review,
            Some("example/other")
        )
        .await
        .is_none(),
        "kind and repo hold together"
    );
    let taken = kind_claim(store, &room, room.author, TaskKind::Change, Some(REPO))
        .await
        .expect("a change seat is handed the open change");
    assert_eq!(taken.id, change);
}

/// The workspace queue depth `room`'s third-family seat reads, narrowed by
/// `repo` and `kind` and by no engine.
async fn depth(
    store: &dyn Store,
    room: &Room,
    repo: Option<&str>,
    kind: Option<TaskKind>,
) -> QueueDepth {
    store
        .workspace_queue_depth(room.ws, room.third, repo, None, kind)
        .await
        .expect("depth")
}

/// Queue depth with `kind = review` counts only reviews, with or without a
/// repository; `kind = change` counts a thread that never named its kind as
/// a change, as claim-next reads it.
async fn the_queue_depth_by_kind_counts_only_that_kind(store: &dyn Store, pool: &Pool) {
    let room = room(store, pool, "kind-depth").await;
    for (title, path, engine) in [
        ("unattended change", "src/", TaskEngine::Unattended),
        ("session change", "docs/", TaskEngine::Session),
    ] {
        change_into_review(
            store,
            pool,
            &room,
            title,
            Some(exclusive(path)),
            TaskAttributes {
                kind: TaskKind::Change,
                engine: Some(engine),
                repo: Some(REPO.into()),
                ..TaskAttributes::default()
            },
        )
        .await;
    }
    open_change(store, &room, "open change", TaskEngine::Session).await;
    store
        .create_thread(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some("never described".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("bare thread");

    let all = depth(store, &room, None, None).await;
    assert_eq!(all.open, 4, "two reviews, the open change, the bare thread");
    let reviews = depth(store, &room, None, Some(TaskKind::Review)).await;
    assert_eq!((reviews.open, reviews.ready), (2, 2), "{reviews:?}");
    let repo_reviews = depth(store, &room, Some(REPO), Some(TaskKind::Review)).await;
    assert_eq!(
        (repo_reviews.open, repo_reviews.ready),
        (2, 2),
        "{repo_reviews:?}"
    );
    let changes = depth(store, &room, None, Some(TaskKind::Change)).await;
    assert_eq!(
        changes.open, 2,
        "the open change and the bare thread: {changes:?}"
    );
    let repo_changes = depth(store, &room, Some(REPO), Some(TaskKind::Change)).await;
    assert_eq!(
        repo_changes.open, 1,
        "only the change with a footprint: {repo_changes:?}"
    );
    let plans = depth(store, &room, None, Some(TaskKind::Plan)).await;
    assert_eq!(plans.open, 0, "{plans:?}");
}

async fn claimed_by_kind(store: &dyn Store, pool: &Pool) {
    a_review_seat_claims_every_review_in_its_repository_whatever_the_engine(store, pool).await;
    the_queue_depth_by_kind_counts_only_that_kind(store, pool).await;
}

#[tokio::test]
async fn a_review_seat_claims_and_counts_by_kind_on_sqlite() {
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
    claimed_by_kind(&store, &Pool::Sqlite(pool)).await;
}

#[tokio::test]
async fn a_review_seat_claims_and_counts_by_kind_on_postgres() {
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
    claimed_by_kind(&store, &Pool::Postgres(pool)).await;
}
