//! Cluster 305: the mail-outbox worker. An enqueued email is delivered by
//! `mail_worker::sweep_once` and not re-sent; a failed send is *rescheduled*
//! (not dropped, not dead-lettered on the first failure), so a transient SMTP
//! outage survives.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{mail_worker, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::NewMailOutbox;
use sqlx::sqlite::SqlitePoolOptions;

struct CountingMailer {
    attempts: AtomicUsize,
    fail: bool,
    sent: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl maidan_server::mail::MailTransport for CountingMailer {
    async fn send(
        &self,
        to: &str,
        _subject: &str,
        _body: &str,
    ) -> Result<(), maidan_server::mail::MailError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(maidan_server::mail::MailError::Send(
                "simulated outage".into(),
            ));
        }
        self.sent.lock().unwrap().push(to.to_string());
        Ok(())
    }
}

async fn state_with(mailer: Arc<CountingMailer>) -> (AppState, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    state.attach_mail(mailer);
    (state, store)
}

fn queued(to: &str) -> NewMailOutbox {
    NewMailOutbox {
        to_address: to.into(),
        subject: "s".into(),
        body: "b".into(),
    }
}

#[tokio::test]
async fn worker_delivers_a_queued_mail_once() {
    let mailer = Arc::new(CountingMailer {
        attempts: AtomicUsize::new(0),
        fail: false,
        sent: Mutex::new(Vec::new()),
    });
    let (state, store) = state_with(mailer.clone()).await;
    store.enqueue_mail(queued("a@example.com")).await.unwrap();

    let stats = mail_worker::sweep_once(&state).await;
    assert_eq!(stats.sent, 1);
    assert_eq!(mailer.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(mailer.sent.lock().unwrap().as_slice(), ["a@example.com"]);

    // Delivered -> not re-claimed on the next sweep.
    let again = mail_worker::sweep_once(&state).await;
    assert_eq!(again.sent, 0);
    assert_eq!(
        mailer.attempts.load(Ordering::SeqCst),
        1,
        "a delivered mail is not re-sent"
    );
}

#[tokio::test]
async fn worker_reschedules_on_failure_instead_of_dropping() {
    let mailer = Arc::new(CountingMailer {
        attempts: AtomicUsize::new(0),
        fail: true,
        sent: Mutex::new(Vec::new()),
    });
    let (state, store) = state_with(mailer.clone()).await;
    store.enqueue_mail(queued("a@example.com")).await.unwrap();

    // A first failure reschedules (attempts 1 < max), it does not dead-letter.
    let stats = mail_worker::sweep_once(&state).await;
    assert_eq!(stats.retried, 1);
    assert_eq!(stats.dead, 0);
    assert_eq!(mailer.attempts.load(Ordering::SeqCst), 1);

    // Rescheduled with backoff (~30s out), so an immediate re-sweep does nothing —
    // the mail is retained for a later retry, not dropped and not dead-lettered.
    let again = mail_worker::sweep_once(&state).await;
    assert_eq!(again.retried, 0);
    assert_eq!(again.sent, 0);
    assert_eq!(
        mailer.attempts.load(Ordering::SeqCst),
        1,
        "not retried until the backoff elapses"
    );
    assert_eq!(
        store.count_dead_mail().await.unwrap(),
        0,
        "a single failure never dead-letters"
    );
}
