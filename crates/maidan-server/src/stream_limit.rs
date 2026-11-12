//! A ceiling on the event streams one credential holds open at once.
//!
//! A server-sent event stream answers its headers at once and then runs for as
//! long as the client stays, so the in-flight ceiling (`crate::load_shed`)
//! never counts it, and the rate limiter caps only how fast streams open, not
//! how many stay open. One token could open streams without bound, each
//! holding a bus subscription and a credential re-check, none of them visible
//! to any metric (RED-005). Every SSE route that stays open takes a
//! [`StreamSlot`] before it subscribes to anything, and the stream gives the
//! slot back when it ends.
//!
//! A credential is the API token behind the request, sent as a bearer or
//! behind a session made from it; a caller with no token counts as its
//! member, so people sharing one address do not share a ceiling.
//! `MAIDAN_MAX_STREAMS_PER_CREDENTIAL` sets the ceiling (default 64, `0` off).
//! The gauge `maidan_sse_streams` counts the open streams and
//! `maidan_sse_streams_refused_total` the refusals. `/ws/subscribe` keeps its
//! own instance-wide ceiling (`MAIDAN_MAX_WS_CONNECTIONS`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::{Stream, StreamExt as _};
use maidan_auth::AuthContext;
use maidan_types::{ApiTokenId, MemberId};
use metrics::counter;

use crate::error::ApiError;

/// Default ceiling on one credential's open streams.
pub const DEFAULT_MAX_STREAMS_PER_CREDENTIAL: usize = 64;

/// Whose ceiling a stream counts against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Credential {
    Token(ApiTokenId),
    Member(MemberId),
}

#[derive(Debug, Default)]
struct Open {
    by_credential: Mutex<HashMap<Credential, usize>>,
    total: AtomicUsize,
}

/// Open streams per credential. Cloning shares the counts.
#[derive(Clone, Debug)]
pub struct StreamLimit {
    max: usize,
    open: Arc<Open>,
}

/// One open stream's place under its credential's ceiling, given back on drop.
#[derive(Debug)]
pub struct StreamSlot {
    // Never read: dropping it gives the slot back.
    _held: Option<Held>,
}

#[derive(Debug)]
struct Held {
    open: Arc<Open>,
    credential: Credential,
}

impl StreamLimit {
    /// A ceiling of `max` open streams per credential; `0` turns it off.
    pub fn new(max: usize) -> Self {
        Self {
            max,
            open: Arc::default(),
        }
    }

    /// Read `MAIDAN_MAX_STREAMS_PER_CREDENTIAL`.
    pub fn from_env() -> Self {
        Self::new(parse_max(
            std::env::var("MAIDAN_MAX_STREAMS_PER_CREDENTIAL").ok(),
        ))
    }

    /// Take a slot for one stream opened by `auth`, or refuse with a `429`
    /// when that credential already holds the ceiling. Auth-disabled callers
    /// are not counted: there is no credential to hold to it.
    pub fn open(&self, auth: &AuthContext) -> Result<StreamSlot, ApiError> {
        if self.max == 0 || auth.bypass {
            return Ok(StreamSlot { _held: None });
        }
        let credential = match auth.token_id() {
            Some(token_id) => Credential::Token(token_id),
            None => Credential::Member(auth.actor_id()),
        };
        // A poisoned lock still holds consistent counts: every writer changes
        // one entry by one.
        let mut by_credential = self
            .open
            .by_credential
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let count = by_credential.entry(credential).or_default();
        if *count >= self.max {
            counter!("maidan_sse_streams_refused_total").increment(1);
            return Err(ApiError::TooManyRequests(format!(
                "this credential already holds {} open event streams; close one first",
                self.max
            )));
        }
        *count += 1;
        self.open.total.fetch_add(1, Ordering::Relaxed);
        Ok(StreamSlot {
            _held: Some(Held {
                open: Arc::clone(&self.open),
                credential,
            }),
        })
    }

    /// Streams open right now, across every credential.
    pub fn open_streams(&self) -> usize {
        self.open.total.load(Ordering::Relaxed)
    }
}

impl Default for StreamLimit {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_STREAMS_PER_CREDENTIAL)
    }
}

impl StreamSlot {
    /// `inner`, holding this slot until the stream ends or is dropped.
    pub fn hold<S>(self, inner: S) -> impl Stream<Item = S::Item> + Send + 'static + use<S>
    where
        S: Stream + Send + 'static,
    {
        inner.map(move |item| {
            let _held = &self;
            item
        })
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut by_credential = self
            .open
            .by_credential
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // The entry goes with its last stream, so the map holds only the
        // credentials streaming now.
        if let Some(count) = by_credential.get_mut(&self.credential) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                by_credential.remove(&self.credential);
            }
        }
        self.open.total.fetch_sub(1, Ordering::Relaxed);
    }
}

/// An unset or unparsable value keeps the default; `0` turns the ceiling off.
fn parse_max(raw: Option<String>) -> usize {
    raw.and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_MAX_STREAMS_PER_CREDENTIAL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::WorkspaceId;

    fn token(token: ApiTokenId) -> AuthContext {
        AuthContext::from_token(
            token,
            MemberId(uuid::Uuid::now_v7()),
            WorkspaceId(uuid::Uuid::now_v7()),
            Vec::new(),
        )
    }

    #[test]
    fn a_credential_at_its_ceiling_is_refused_and_another_is_not() {
        let limit = StreamLimit::new(2);
        let a = token(ApiTokenId(uuid::Uuid::now_v7()));
        let b = token(ApiTokenId(uuid::Uuid::now_v7()));
        let first = limit.open(&a).unwrap();
        let _second = limit.open(&a).unwrap();
        assert!(matches!(limit.open(&a), Err(ApiError::TooManyRequests(_))));
        let _other = limit.open(&b).unwrap();
        assert_eq!(limit.open_streams(), 3);
        drop(first);
        assert_eq!(limit.open_streams(), 2);
        assert!(
            limit.open(&a).is_ok(),
            "a closed stream gives its slot back"
        );
    }

    #[test]
    fn a_ceiling_of_zero_and_an_auth_disabled_caller_are_not_counted() {
        let off = StreamLimit::new(0);
        let a = token(ApiTokenId(uuid::Uuid::now_v7()));
        let _slots: Vec<_> = (0..5).map(|_| off.open(&a).unwrap()).collect();
        assert_eq!(off.open_streams(), 0);
        let on = StreamLimit::new(1);
        let _slots: Vec<_> = (0..5)
            .map(|_| on.open(&AuthContext::bypass()).unwrap())
            .collect();
        assert_eq!(on.open_streams(), 0);
    }

    #[test]
    fn the_ceiling_parses_and_falls_back() {
        assert_eq!(parse_max(None), DEFAULT_MAX_STREAMS_PER_CREDENTIAL);
        assert_eq!(
            parse_max(Some("nope".into())),
            DEFAULT_MAX_STREAMS_PER_CREDENTIAL
        );
        assert_eq!(parse_max(Some(" 8 ".into())), 8);
        assert_eq!(parse_max(Some("0".into())), 0);
    }

    #[tokio::test]
    async fn a_held_stream_keeps_its_slot_until_it_is_dropped() {
        let limit = StreamLimit::new(1);
        let a = token(ApiTokenId(uuid::Uuid::now_v7()));
        let stream = limit.open(&a).unwrap().hold(futures::stream::iter([1, 2]));
        assert!(limit.open(&a).is_err(), "the held stream counts");
        let items: Vec<_> = stream.collect().await;
        assert_eq!(items, [1, 2]);
        assert!(
            limit.open(&a).is_ok(),
            "a finished stream gives its slot back"
        );
    }
}
