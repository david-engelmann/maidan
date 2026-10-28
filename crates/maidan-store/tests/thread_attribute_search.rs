//! Find threads by task attribute (`find_threads_for_workspace`): the
//! attribute filters match as the attributes read, the search never
//! leaves the workspace it was given, and a reader sees only the threads
//! they may read — identically on both backends.

use std::time::Duration;

use maidan_store::store::{ThreadAttributeFilter, WorkspaceThreadSearch};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

async fn member(store: &dyn Store, workspace_id: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id
}

async fn channel(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    name: &str,
    private: bool,
) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id,
            name: name.into(),
            topic: None,
            private,
        })
        .await
        .unwrap()
        .id
}

async fn task(
    store: &dyn Store,
    channel_id: ChannelId,
    title: &str,
    attributes: Option<TaskAttributes>,
) -> ThreadId {
    let new = NewThread {
        channel_id,
        parent_thread_id: None,
        title: Some(title.into()),
        description: Some(format!("{title} brief")),
        created_by: None,
    };
    match attributes {
        Some(attributes) => {
            store
                .create_thread_with_footprint(new, None, Some(attributes), None)
                .await
                .unwrap()
                .0
                .id
        }
        None => store.create_thread(new).await.unwrap().id,
    }
}

fn search(filter: ThreadAttributeFilter) -> WorkspaceThreadSearch {
    WorkspaceThreadSearch {
        filter,
        limit: 100,
        ..WorkspaceThreadSearch::default()
    }
}

async fn ids(store: &dyn Store, ws: WorkspaceId, search: WorkspaceThreadSearch) -> Vec<ThreadId> {
    store
        .find_threads_for_workspace(ws, search)
        .await
        .unwrap()
        .iter()
        .map(|t| t.id)
        .collect()
}

async fn assert_attribute_search(store: &dyn Store) {
    let ws_a = workspace(store, "search-a").await;
    let ws_b = workspace(store, "search-b").await;
    let alice = member(store, ws_a, "alice").await;
    let _bob = member(store, ws_b, "bob").await;
    let open_a = channel(store, ws_a, "open-a", false).await;
    let private_a = channel(store, ws_a, "private-a", true).await;
    let open_b = channel(store, ws_b, "open-b", false).await;

    let pi_issue = TaskAttributes {
        kind: TaskKind::Change,
        engine: Some(TaskEngine::Pi),
        linear_issue: Some("ENG-100".into()),
        tier: Some(TaskTier::A),
        lane: Some(TaskLane::Batch),
        ..TaskAttributes::default()
    };
    let t_pi = task(store, open_a, "pi issue", Some(pi_issue.clone())).await;
    let t_session = task(
        store,
        open_a,
        "session issue",
        Some(TaskAttributes {
            kind: TaskKind::Change,
            engine: Some(TaskEngine::Session),
            linear_issue: Some("ENG-200".into()),
            tier: Some(TaskTier::B),
            ..TaskAttributes::default()
        }),
    )
    .await;
    // No attributes at all: reads as the defaults.
    let t_plain = task(store, open_a, "plain", None).await;
    let t_review = task(
        store,
        open_a,
        "review of the issue",
        Some(TaskAttributes {
            kind: TaskKind::Review,
            engine: Some(TaskEngine::Pi),
            linear_issue: Some("ENG-100".into()),
            ..TaskAttributes::default()
        }),
    )
    .await;
    // Same attribute values as t_pi, but in a private channel alice is
    // not a member of, and in another workspace entirely.
    let t_private = task(store, private_a, "private pi issue", Some(pi_issue.clone())).await;
    let t_other_ws = task(store, open_b, "other workspace pi issue", Some(pi_issue)).await;

    store
        .add_reference(NewReference {
            src_kind: RefSide::Thread,
            src_id: t_review.0,
            dst_kind: RefSide::Thread,
            dst_id: t_pi.0,
            relation: RelationKind::from_wire("twin_of"),
        })
        .await
        .unwrap();

    let by_issue = ThreadAttributeFilter {
        linear_issue: Some("ENG-100".into()),
        ..Default::default()
    };
    // Unrestricted: every ENG-100 thread in workspace A, never B's.
    let found = ids(store, ws_a, search(by_issue.clone())).await;
    assert_eq!(found, vec![t_pi, t_review, t_private], "issue filter, ws A");
    // The same filter in workspace B finds only B's thread.
    let found_b = ids(store, ws_b, search(by_issue)).await;
    assert_eq!(found_b, vec![t_other_ws], "issue filter, ws B");

    // A reader who is not in the private channel never sees its threads.
    let mut as_alice = search(ThreadAttributeFilter {
        engine: Some(TaskEngine::Pi),
        ..Default::default()
    });
    as_alice.reader_id = Some(alice);
    let found = ids(store, ws_a, as_alice).await;
    assert_eq!(found, vec![t_pi, t_review], "engine filter as a reader");

    // Unset fields match nothing: t_plain has no tier, so tier B is only
    // the thread that said B.
    let found = ids(
        store,
        ws_a,
        search(ThreadAttributeFilter {
            tier: Some(TaskTier::B),
            ..Default::default()
        }),
    )
    .await;
    assert_eq!(found, vec![t_session], "tier filter matches stored values");

    // The defaults a thread reads with do match: kind defaults to change
    // and threat_model to false, for described and undescribed threads.
    let found = ids(
        store,
        ws_a,
        search(ThreadAttributeFilter {
            kind: Some(TaskKind::Change),
            ..Default::default()
        }),
    )
    .await;
    assert_eq!(
        found,
        vec![t_pi, t_session, t_plain, t_private],
        "kind filter matches the default"
    );
    let found = ids(
        store,
        ws_a,
        search(ThreadAttributeFilter {
            threat_model: Some(false),
            ..Default::default()
        }),
    )
    .await;
    assert_eq!(found.len(), 5, "threat_model false matches every A thread");

    // References filter the same way: the review thread is the twin of
    // the pi thread.
    let found = ids(
        store,
        ws_a,
        search(ThreadAttributeFilter {
            reference: Some(("twin_of".into(), t_pi)),
            ..Default::default()
        }),
    )
    .await;
    assert_eq!(found, vec![t_review], "ref.twin_of filter");
    // The relation is part of the match: no thread duplicates t_pi.
    let found = ids(
        store,
        ws_a,
        search(ThreadAttributeFilter {
            reference: Some(("duplicates".into(), t_pi)),
            ..Default::default()
        }),
    )
    .await;
    assert!(
        found.is_empty(),
        "ref.duplicates matches only that relation"
    );
    // A destination in another workspace probes nothing: workspace B's
    // search for twins of A's thread is empty.
    let found = ids(
        store,
        ws_b,
        search(ThreadAttributeFilter {
            reference: Some(("twin_of".into(), t_pi)),
            ..Default::default()
        }),
    )
    .await;
    assert!(found.is_empty(), "ref.twin_of across workspaces");

    // Filters combine: the review thread shares the issue and engine but
    // not the kind. The returned threads carry their attributes.
    let found = store
        .find_threads_for_workspace(
            ws_a,
            search(ThreadAttributeFilter {
                kind: Some(TaskKind::Change),
                linear_issue: Some("ENG-100".into()),
                engine: Some(TaskEngine::Pi),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        found.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![t_pi, t_private]
    );
    assert_eq!(
        found[0]
            .attributes
            .as_ref()
            .unwrap()
            .linear_issue
            .as_deref(),
        Some("ENG-100")
    );

    // Keyset pagination walks the matches, newest first on request.
    let page1 = store
        .find_threads_for_workspace(
            ws_a,
            WorkspaceThreadSearch {
                filter: ThreadAttributeFilter {
                    engine: Some(TaskEngine::Pi),
                    ..Default::default()
                },
                limit: 2,
                newest_first: true,
                ..WorkspaceThreadSearch::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page1.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![t_private, t_review]
    );
    let page2 = store
        .find_threads_for_workspace(
            ws_a,
            WorkspaceThreadSearch {
                filter: ThreadAttributeFilter {
                    engine: Some(TaskEngine::Pi),
                    ..Default::default()
                },
                after: page1.last().map(|t| t.id),
                limit: 2,
                newest_first: true,
                ..WorkspaceThreadSearch::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page2.iter().map(|t| t.id).collect::<Vec<_>>(), vec![t_pi]);

    // The filter parser the surfaces share: a typo'd field or value is an
    // error, never a silent match-everything.
    assert!(
        ThreadAttributeFilter::from_pairs(vec![("linear_issue".into(), "ENG-100".into())]).is_ok()
    );
    assert!(
        ThreadAttributeFilter::from_pairs(vec![("linear_issue".into(), "not-a-key".into())])
            .is_err()
    );
    assert!(ThreadAttributeFilter::from_pairs(vec![("engnie".into(), "pi".into())]).is_err());
    assert!(ThreadAttributeFilter::from_pairs(vec![("engine".into(), "robot".into())]).is_err());
}

#[tokio::test]
async fn attribute_search_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    assert_attribute_search(&store).await;
}

#[tokio::test]
async fn attribute_search_postgres() {
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
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::for_tests(pool);
    assert_attribute_search(&store).await;
}
