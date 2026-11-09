//! Operator alerts outlive the process that raised them. The hosted
//! container sleeps after 15 idle minutes and wakes a fresh process, so a
//! dedupe held in memory posted "started" on every wake and reposted a
//! standing condition after each one. A restart here is a fresh
//! [`AlertLog`] over the same store.

mod common;

use std::sync::Arc;

use maidan_server::ops_alert::{AlertLog, REPEAT_SECS};
use maidan_store::{prelude::*, run_sqlite_migrations};

const DENIED: &str = "github-checks-denied:acme/app";

async fn sqlite_pool() -> sqlx::SqlitePool {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    pool
}

async fn sqlite() -> Arc<dyn Store> {
    Arc::new(SqliteStore::for_tests(sqlite_pool().await))
}

type Container = testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>;

async fn postgres() -> Option<(Container, Arc<dyn Store>)> {
    let (container, pool) = common::postgres_pool().await?;
    Some((
        container,
        Arc::new(PostgresStore::for_tests(pool)) as Arc<dyn Store>,
    ))
}

async fn the_same_build_waking_twice_announces_itself_once(store: Arc<dyn Store>) {
    let first = AlertLog::new(Some(store.clone()));
    assert!(
        first.may_raise("boot:C0123ABCDE", "v438.0.0", 0).await,
        "the first boot ever posts"
    );
    let woken = AlertLog::new(Some(store.clone()));
    assert!(
        !woken.may_raise("boot:C0123ABCDE", "v438.0.0", 3_000).await,
        "a wake of the same build is silent"
    );
    let woken_again = AlertLog::new(Some(store.clone()));
    assert!(
        !woken_again
            .may_raise("boot:C0123ABCDE", "v438.0.0", 6_000)
            .await
    );
    let deployed = AlertLog::new(Some(store));
    assert!(
        deployed
            .may_raise("boot:C0123ABCDE", "v439.0.0", 9_000)
            .await,
        "a new build posts"
    );
}

async fn a_condition_holding_across_a_restart_posts_once(store: Arc<dyn Store>) {
    let before = AlertLog::new(Some(store.clone()));
    assert!(
        before.may_raise(DENIED, "standing", 0).await,
        "the outage posts"
    );
    assert!(
        !before.may_raise(DENIED, "standing", REPEAT_SECS * 4).await,
        "not again every 15 minutes while the process is awake"
    );
    let after = AlertLog::new(Some(store));
    assert!(
        !after.may_raise(DENIED, "standing", REPEAT_SECS * 8).await,
        "nor again after the process restarts"
    );
}

async fn a_condition_that_clears_and_comes_back_posts_again(store: Arc<dyn Store>) {
    let log = AlertLog::new(Some(store.clone()));
    assert!(log.may_raise(DENIED, "standing", 0).await);
    let restarted = AlertLog::new(Some(store.clone()));
    assert!(
        restarted.may_resolve(DENIED).await,
        "the clear owes a resolved line, even from another process"
    );
    assert!(
        !restarted.may_resolve(DENIED).await,
        "a condition that stays clear is resolved once"
    );
    assert!(
        restarted.may_raise(DENIED, "standing", 60).await,
        "the condition coming back posts again, inside the old repeat window"
    );
    assert!(
        !AlertLog::new(Some(store))
            .may_raise(DENIED, "standing", 120)
            .await
    );
}

#[tokio::test]
async fn the_same_build_waking_twice_announces_itself_once_sqlite() {
    the_same_build_waking_twice_announces_itself_once(sqlite().await).await;
}

#[tokio::test]
async fn the_same_build_waking_twice_announces_itself_once_postgres() {
    let Some((_container, store)) = postgres().await else {
        return;
    };
    the_same_build_waking_twice_announces_itself_once(store).await;
}

#[tokio::test]
async fn a_condition_holding_across_a_restart_posts_once_sqlite() {
    a_condition_holding_across_a_restart_posts_once(sqlite().await).await;
}

#[tokio::test]
async fn a_condition_holding_across_a_restart_posts_once_postgres() {
    let Some((_container, store)) = postgres().await else {
        return;
    };
    a_condition_holding_across_a_restart_posts_once(store).await;
}

#[tokio::test]
async fn a_condition_that_clears_and_comes_back_posts_again_sqlite() {
    a_condition_that_clears_and_comes_back_posts_again(sqlite().await).await;
}

#[tokio::test]
async fn a_condition_that_clears_and_comes_back_posts_again_postgres() {
    let Some((_container, store)) = postgres().await else {
        return;
    };
    a_condition_that_clears_and_comes_back_posts_again(store).await;
}

/// A raise Slack refused is taken back, so the next pass posts it, even
/// from a fresh process: a retry queued in memory would die with it.
async fn a_raise_slack_refused_posts_on_the_next_pass_even_after_a_restart(store: Arc<dyn Store>) {
    let refused = AlertLog::new(Some(store.clone()));
    assert!(refused.may_raise(DENIED, "standing", 0).await);
    assert!(refused.unclaim(DENIED).await);
    assert!(
        refused.may_raise(DENIED, "standing", 60).await,
        "the next pass posts it"
    );
    assert!(refused.unclaim(DENIED).await);
    assert!(
        AlertLog::new(Some(store))
            .may_raise(DENIED, "standing", 120)
            .await,
        "and so does a fresh process"
    );
}

#[tokio::test]
async fn a_raise_slack_refused_posts_on_the_next_pass_even_after_a_restart_sqlite() {
    a_raise_slack_refused_posts_on_the_next_pass_even_after_a_restart(sqlite().await).await;
}

#[tokio::test]
async fn a_raise_slack_refused_posts_on_the_next_pass_even_after_a_restart_postgres() {
    let Some((_container, store)) = postgres().await else {
        return;
    };
    a_raise_slack_refused_posts_on_the_next_pass_even_after_a_restart(store).await;
}

/// A store that does not answer must not cost a post: the condition falls
/// back to the in-memory window it had before. Its resolved line waits for
/// the store, so the pass that clears the row posts it once.
#[tokio::test]
async fn an_unreachable_store_falls_back_to_the_in_memory_window() {
    let pool = sqlite_pool().await;
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    pool.close().await;
    let log = AlertLog::new(Some(store));
    assert!(
        log.may_raise(DENIED, "standing", 0).await,
        "the alert is not lost"
    );
    assert!(!log.may_raise(DENIED, "standing", REPEAT_SECS - 1).await);
    assert!(log.may_raise(DENIED, "standing", REPEAT_SECS).await);
    assert!(
        !log.may_resolve(DENIED).await,
        "the resolved line waits for the store"
    );
    assert!(log.may_raise("boot:C0123ABCDE", "v438.0.0", 0).await);

    let memory = AlertLog::new(None);
    assert!(memory.may_raise(DENIED, "standing", 0).await);
    assert!(memory.may_resolve(DENIED).await);
    assert!(!memory.may_resolve(DENIED).await);
    assert!(
        memory.may_raise(DENIED, "standing", 60).await,
        "back again, it posts"
    );
}

/// A store that fails just after a post does not repost the condition on
/// the next pass: the post is in the in-memory window too.
#[tokio::test]
async fn a_store_that_fails_after_a_post_does_not_repost_inside_the_window() {
    let pool = sqlite_pool().await;
    let log = AlertLog::new(Some(Arc::new(SqliteStore::for_tests(pool.clone()))));
    assert!(log.may_raise(DENIED, "standing", 0).await);
    pool.close().await;
    assert!(!log.may_raise(DENIED, "standing", 120).await);
    assert!(log.may_raise(DENIED, "standing", REPEAT_SECS).await);
}
