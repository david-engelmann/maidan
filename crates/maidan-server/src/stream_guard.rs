//! Re-checks the credential behind a running server-sent event stream.
//!
//! A stream is authorized once, when it opens. Deactivating a member (or
//! revoking a token) makes every later *request* fail, but a stream makes no
//! further requests, so on its own it would outlive the credential. Every SSE
//! route wraps its stream in [`guard`] (A2A's gRPC `SubscribeToTask` uses
//! [`guard_ending`], which ends with `UNAUTHENTICATED` instead). The wrapper checks the credential again
//! before a frame goes out (at most once per [`RECHECK_GAP`]) and every
//! [`RECHECK_TICK`] while the stream is idle. When the check fails it sends one
//! last `stream_ended` event that gives the reason, then ends the stream.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::Event;
use futures::{Stream, StreamExt as _};
use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{ApiTokenId, MemberId};
use tokio::time::{Instant, Interval, MissedTickBehavior};

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
    /// Auth is disabled: nothing to check.
    Open,
    /// An API token (sent as a bearer, or behind a session made from it).
    /// It must still pass the bearer test, and the members it speaks for must
    /// not be deactivated.
    Token(ApiTokenId),
    /// No token behind the context (e.g. the dev anonymous reader): only the
    /// members are checked.
    Members(Vec<MemberId>),
}

impl StreamCredential {
    pub fn from_auth(auth: &AuthContext) -> Self {
        if auth.bypass {
            return Self::Open;
        }
        match auth.token_id {
            Some(token_id) => Self::Token(token_id),
            None => Self::Members(members_of(auth)),
        }
    }

    /// `Err(reason)` when the stream must end. A failed lookup also ends the
    /// stream, because the check cannot show the credential is still valid.
    /// The client reconnects and goes through the normal check.
    pub async fn recheck(&self, store: &dyn Store) -> Result<(), &'static str> {
        let members = match self {
            Self::Open => return Ok(()),
            Self::Token(token_id) => {
                let ctx = maidan_auth::resolve_token_id(store, *token_id)
                    .await
                    .map_err(|_| "token no longer valid")?;
                members_of(&ctx)
            }
            Self::Members(members) => members.clone(),
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
    let mut members = vec![auth.member_id];
    if auth.actor_id != auth.member_id {
        members.push(auth.actor_id);
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
    ended: bool,
}

impl<T, E: Fn(&'static str) -> T> Guard<T, E> {
    /// Check now. On failure, mark the stream ended and return its last item.
    async fn check(&mut self) -> Option<T> {
        self.last_check = Some(Instant::now());
        match self.credential.recheck(self.store.as_ref()).await {
            Ok(()) => None,
            Err(reason) => {
                self.ended = true;
                Some((self.end_with)(reason))
            }
        }
    }
}

/// Wrap an SSE stream so it ends, with a `stream_ended` frame, once `auth`'s
/// credential no longer holds.
pub fn guard<S>(
    store: Arc<dyn Store>,
    auth: &AuthContext,
    inner: S,
) -> impl Stream<Item = Frame> + Send + 'static
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
) -> impl Stream<Item = T> + Send + 'static
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
        end_with,
    )
}

fn guard_with<T, S, E>(
    store: Arc<dyn Store>,
    credential: StreamCredential,
    inner: S,
    tick_every: Duration,
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
        ended: false,
    };
    futures::stream::unfold(state, |mut g| async move {
        if g.ended {
            return None;
        }
        if g.credential == StreamCredential::Open {
            let next = g.inner.next().await?;
            return Some((next, g));
        }
        loop {
            tokio::select! {
                next = g.inner.next() => {
                    let item = next?;
                    if g.last_check.is_none_or(|at| at.elapsed() >= RECHECK_GAP) {
                        if let Some(last) = g.check().await {
                            return Some((last, g));
                        }
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
    fn bypass_is_open() {
        assert_eq!(
            StreamCredential::from_auth(&AuthContext::bypass()),
            StreamCredential::Open
        );
    }

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

    #[test]
    fn ended_event_names_the_reason() {
        let rendered = format!("{:?}", ended_event("member deactivated"));
        assert!(rendered.contains("stream_ended"), "{rendered}");
        assert!(rendered.contains("member deactivated"), "{rendered}");
    }
}
