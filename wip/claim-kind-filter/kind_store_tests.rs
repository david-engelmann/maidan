
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
        kind_claim(store, &room, room.third, TaskKind::Review, Some("example/other"))
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
        .workspace_queue_depth(room.ws, Some(room.third), repo, None, kind)
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
    assert_eq!((repo_reviews.open, repo_reviews.ready), (2, 2), "{repo_reviews:?}");
    let changes = depth(store, &room, None, Some(TaskKind::Change)).await;
    assert_eq!(changes.open, 2, "the open change and the bare thread: {changes:?}");
    let repo_changes = depth(store, &room, Some(REPO), Some(TaskKind::Change)).await;
    assert_eq!(repo_changes.open, 1, "only the change with a footprint: {repo_changes:?}");
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
