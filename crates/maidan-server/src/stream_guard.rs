//! Re-checks the credential behind a running server-sent event stream.
//!
//! A stream is authorized once, when it opens. Deactivating a member (or
//! revoking a token) makes every later *request* fail, but a stream makes no
//! further requests, so on its own it would outlive the credential. Every SSE
//! route wraps its stream in [`guard`] (A2A's gRPC `SubscribeToTask` uses
//! [`guard_ending`], which ends with `UNAUTHENTICATED` instead). The wrapper checks the credential again
//! before a frame goes out (the first, then at most once per [`RECHECK_GAP`]) and every
//! [`RECHECK_TICK`] while the stream is idle. When the check fails it sends one
//! last `stream_ended` event that gives the reason, then ends the stream.
//!
//! The same wrapper ends the stream, with the reason
//! [`crate::shutdown::SERVER_RESTARTING`], when the server begins to shut
//! down: an open stream would otherwise keep a stopped server running
//! (`crate::shutdown`). The client reconnects to the server that replaces it.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::Event;
use futures::{Stream, StreamExt as _};
use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{ApiTokenId, MemberId, WorkspaceId};
use tokio::sync::watch;
use tokio::time::{Instant, Interval, MissedTickBehavior};

use crate::shutdown::{begun, SERVER_RESTARTING};

/// The shortest time between two checks while frames flow. A burst of frames
/// costs one check, and a deactivated member's stream ends at the first frame
/// after this much time.
pub const RECHECK_GAP: Duration = Duration::from_secs(1);

/// How often an idle stream is checked. This bounds how long a stream with
/// no frames outlives its credential.
pub const RECHECK_TICK: Duration = Duration::from_secs(10);

/// The SSE event name of the terminal frame.
pub const STREAM_ENDED_EVENT: &str = "stream_ended";

/// What opened the stream, kept so it can be checked again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamCredential {
    /// An API token (sent as a bearer, or behind a session made from it).
    /// It must still pass the bearer test, and the members it speaks for must
    /// not be deactivated.
    Token(ApiTokenId),
    /// No token behind the context (e.g. the dev anonymous reader, an OIDC
    /// session): the workspace must not be suspended and the members must
    /// not be deactivated.
    Members {
        workspace_id: WorkspaceId,
        members: Vec<MemberId>,
    },
}

impl StreamCredential {
    pub fn from_auth(auth: &AuthContext) -> Self {
        match auth.token_id() {
            Some(token_id) => Self::Token(token_id),
            None => Self::Members {
                workspace_id: auth.workspace_id(),
                members: members_of(auth),
            },
        }
    }

    /// `Err(reason)` when the stream must end. A failed lookup also ends the
    /// stream, because the check cannot show the credential is still valid.
    /// The client reconnects and goes through the normal check.
    pub async fn recheck(&self, store: &dyn Store) -> Result<(), &'static str> {
        let members = match self {
            Self::Token(token_id) => {
                let ctx = match maidan_auth::resolve_token_id(store, *token_id).await {
                    Ok(ctx) => ctx,
                    Err(maidan_auth::AuthError::Forbidden(_)) => {
                        return Err(maidan_types::WORKSPACE_SUSPENDED)
                    }
                    Err(_) => return Err("token no longer valid"),
                };
                members_of(&ctx)
            }
            Self::Members {
                workspace_id,
                members,
            } => {
                match maidan_auth::ensure_workspace_active(store, *workspace_id).await {
                    Ok(()) => {}
                    Err(maidan_auth::AuthError::Forbidden(_)) => {
                        return Err(maidan_types::WORKSPACE_SUSPENDED)
                    }
                    Err(_) => return Err("credential check failed"),
                }
                members.clone()
            }
        };
        for member in members {
            match store.get_scim_user(member).await {
                Ok(Some(user)) if !user.active => return Err("member deactivated"),
                Ok(_) => {}
                Err(_) => return Err("credential check failed"),
            }
        }
        Ok(())
    }
}

/// The member actions are attributed to, and the delegate presenting them if
/// that is someone else. Either one being deactivated ends the stream.
fn members_of(auth: &AuthContext) -> Vec<MemberId> {
    let mut members = vec![auth.member_id()];
    if auth.actor_id() != auth.member_id() {
        members.push(auth.actor_id());
    }
    members
}

/// The terminal frame: `event: stream_ended`, `data: {"reason": "..."}`.
pub fn ended_event(reason: &str) -> Event {
    Event::default()
        .event(STREAM_ENDED_EVENT)
        .data(serde_json::json!({ "reason": reason }).to_string())
}

type Frame = Result<Event, Infallible>;

struct Guard<T, E> {
    inner: Pin<Box<dyn Stream<Item = T> + Send>>,
    store: Arc<dyn Store>,
    credential: StreamCredential,
    end_with: E,
    /// When the credential was last checked; `None` until the first frame,
    /// which is always checked, so a reconnect on an ended credential does
    /// not get a second's worth of frames first.
    last_check: Option<Instant>,
    tick: Interval,
    /// Reads `true` once the server begins to shut down.
    shutdown: watch::Receiver<bool>,
    ended: bool,
}

impl<T, E: Fn(&'static str) -> T> Guard<T, E> {
    /// Check now. On failure, mark the stream ended and return its last item.
    async fn check(&mut self) -> Option<T> {
        self.last_check = Some(Instant::now());
        match self.credential.recheck(self.store.as_ref()).await {
            Ok(()) => None,
            Err(reason) => Some(self.end(reason)),
        }
    }

    /// Mark the stream ended and make its last item.
    fn end(&mut self, reason: &'static str) -> T {
        self.ended = true;
        (self.end_with)(reason)
    }
}

/// Wrap an SSE stream so it ends, with a `stream_ended` frame, once `auth`'s
/// credential no longer holds.
pub fn guard<S>(
    store: Arc<dyn Store>,
    auth: &AuthContext,
    inner: S,
) -> impl Stream<Item = Frame> + Send + 'static + use<S>
where
    S: Stream<Item = Frame> + Send + 'static,
{
    guard_ending(store, auth, inner, |reason| Ok(ended_event(reason)))
}

/// [`guard`] for any stream: `end_with(reason)` makes its last item (for
/// example a gRPC `Status`).
pub fn guard_ending<T, S, E>(
    store: Arc<dyn Store>,
    auth: &AuthContext,
    inner: S,
    end_with: E,
) -> impl Stream<Item = T> + Send + 'static + use<T, S, E>
where
    T: Send + 'static,
    S: Stream<Item = T> + Send + 'static,
    E: Fn(&'static str) -> T + Send + Sync + 'static,
{
    guard_with(
        store,
        StreamCredential::from_auth(auth),
        inner,
        RECHECK_TICK,
        crate::shutdown::subscribe(),
        end_with,
    )
}

fn guard_with<T, S, E>(
    store: Arc<dyn Store>,
    credential: StreamCredential,
    inner: S,
    tick_every: Duration,
    shutdown: watch::Receiver<bool>,
    end_with: E,
) -> impl Stream<Item = T> + Send + 'static
where
    T: Send + 'static,
    S: Stream<Item = T> + Send + 'static,
    E: Fn(&'static str) -> T + Send + Sync + 'static,
{
    let mut tick = tokio::time::interval_at(Instant::now() + tick_every, tick_every);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let state = Guard {
        inner: Box::pin(inner),
        store,
        credential,
        end_with,
        last_check: None,
        tick,
        shutdown,
        ended: false,
    };
    futures::stream::unfold(state, |mut g| async move {
        if g.ended {
            return None;
        }
        loop {
            tokio::select! {
                biased;
                () = begun(&mut g.shutdown) => {
                    let last = g.end(SERVER_RESTARTING);
                    return Some((last, g));
                }
                next = g.inner.next() => {
                    let item = next?;
                    if g.last_check.is_none_or(|at| at.elapsed() >= RECHECK_GAP)
                        && let Some(last) = g.check().await {
                            return Some((last, g));
                        }
                    return Some((item, g));
                }
                _ = g.tick.tick() => {
                    if let Some(last) = g.check().await {
                        return Some((last, g));
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_context_keeps_the_token() {
        let token = ApiTokenId(uuid::Uuid::new_v4());
        let auth = AuthContext::from_token(
            token,
            MemberId(uuid::Uuid::new_v4()),
            maidan_types::WorkspaceId(uuid::Uuid::new_v4()),
            Vec::new(),
        );
        assert_eq!(
            StreamCredential::from_auth(&auth),
            StreamCredential::Token(token)
        );
    }

    async fn unused_store() -> Arc<dyn Store> {
        // Neither credential below reaches the store before shutdown.
        let pool = maidan_search::sqlite_pool_options()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        Arc::new(maidan_store::SqliteStore::for_tests(pool))
    }

    async fn ends_on_shutdown(credential: StreamCredential) {
        let (tx, rx) = watch::channel(false);
        // A stream with nothing to send, as an idle notification stream is.
        let inner = futures::stream::pending::<Frame>();
        let mut stream = Box::pin(guard_with(
            unused_store().await,
            credential,
            inner,
            Duration::from_secs(3600),
            rx,
            |reason| Ok(ended_event(reason)),
        ));
        let idle = tokio::time::timeout(Duration::from_millis(20), stream.next()).await;
        assert!(
            idle.is_err(),
            "an idle stream sent something before shutdown"
        );
        tx.send_replace(true);
        let last = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("the stream ends once shutdown begins")
            .expect("with one last frame")
            .unwrap();
        let rendered = format!("{last:?}");
        assert!(rendered.contains(STREAM_ENDED_EVENT), "{rendered}");
        assert!(rendered.contains(SERVER_RESTARTING), "{rendered}");
        let after = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("the stream closes after its last frame");
        assert!(after.is_none(), "frames after stream_ended");
    }

    #[tokio::test]
    async fn a_checked_stream_ends_when_the_server_shuts_down() {
        ends_on_shutdown(StreamCredential::Members {
            workspace_id: WorkspaceId(uuid::Uuid::now_v7()),
            members: vec![MemberId(uuid::Uuid::now_v7())],
        })
        .await;
    }

    #[test]
    fn ended_event_names_the_reason() {
        let rendered = format!("{:?}", ended_event("member deactivated"));
        assert!(rendered.contains("stream_ended"), "{rendered}");
        assert!(rendered.contains("member deactivated"), "{rendered}");
    }
}
