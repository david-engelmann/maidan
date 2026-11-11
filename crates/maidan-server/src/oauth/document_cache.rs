//! Client metadata documents this process fetched and validated recently
//! (`docs/OAuth.md`, "Client metadata documents"; N1, decided 2026-11-10).
//!
//! Without it every authorize, consent page and token request fetched the
//! client's document again, so one connector sign-in cost up to three
//! fetches of a listed host, and a flood of authorize requests became a
//! flood of fetches to claude.ai and the other listed hosts. The CIMD draft
//! says a server SHOULD cache.
//!
//! **It saves fetches; it is not an authority.** Only a document that passed
//! every check in [`super::registry::resolve_client`] is held: a listed host
//! at a public address, `https`, no redirects, under the size cap, a
//! `client_id` equal to its URL and a cleaned name. A refused or failed fetch
//! is never held, so the next request fetches again. A hit still goes through
//! the pre-registered lookup, the host list and the client row's upsert,
//! which refuses a revoked row, exactly as a fetch does.
//!
//! **Shape.** Keyed by the document's URL, held for [`DOCUMENT_TTL`] from the
//! fetch, at most [`MAX_CACHED_DOCUMENTS`] at once, the oldest evicted first.
//! Memory per process: replicas each fetch once per five minutes, and a
//! restart starts empty. Sharing it would put a store round trip where a
//! fetch used to be, for a document that is public anyway.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::retry_budget::{Clock, SystemClock};

/// How long a validated document is used before it is fetched again. A
/// client that changes its redirect URIs or scope is seen within this long.
pub const DOCUMENT_TTL: Duration = Duration::from_secs(5 * 60);

/// The most documents held at once. Only a document whose `client_id` is its
/// own URL on a listed host is held, so real instances hold a handful; the
/// cap keeps a listed host that serves many valid documents from growing the
/// map without bound.
pub const MAX_CACHED_DOCUMENTS: usize = 1024;

/// What a validated document says about its client: everything the client
/// row is refreshed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedDocument {
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub allowed_scopes: Vec<String>,
}

pub struct DocumentCache {
    clock: Arc<dyn Clock>,
    entries: Mutex<HashMap<String, (Instant, ValidatedDocument)>>,
}

impl DocumentCache {
    /// The cache the server runs with, on the system clock.
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    /// A cache on `clock`, so a test can move past [`DOCUMENT_TTL`] without
    /// waiting for it.
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The document validated at `url` within [`DOCUMENT_TTL`], if any. An
    /// older one is dropped, so the caller fetches it again.
    pub fn get(&self, url: &str) -> Option<ValidatedDocument> {
        let now = self.clock.now();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        match entries.get(url) {
            Some((fetched_at, doc)) if fresh(*fetched_at, now) => Some(doc.clone()),
            Some(_) => {
                entries.remove(url);
                None
            }
            None => None,
        }
    }

    /// Hold `doc`, just validated at `url`. When the cache is full, expired
    /// entries go first and then the oldest.
    pub fn insert(&self, url: &str, doc: ValidatedDocument) {
        let now = self.clock.now();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if !entries.contains_key(url) && entries.len() >= MAX_CACHED_DOCUMENTS {
            entries.retain(|_, (fetched_at, _)| fresh(*fetched_at, now));
            if entries.len() >= MAX_CACHED_DOCUMENTS
                && let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, (fetched_at, _))| *fetched_at)
                    .map(|(key, _)| key.clone())
            {
                entries.remove(&oldest);
            }
        }
        entries.insert(url.to_string(), (now, doc));
    }

    /// How many documents are held.
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for DocumentCache {
    fn default() -> Self {
        Self::new()
    }
}

fn fresh(fetched_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(fetched_at) < DOCUMENT_TTL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry_budget::ManualClock;

    fn doc(name: &str) -> ValidatedDocument {
        ValidatedDocument {
            name: name.to_string(),
            redirect_uris: vec!["https://client.example/callback".to_string()],
            allowed_scopes: vec!["workspace:read".to_string()],
        }
    }

    fn cache() -> (DocumentCache, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new());
        (DocumentCache::with_clock(clock.clone()), clock)
    }

    #[test]
    fn a_document_is_served_for_five_minutes_and_then_dropped() {
        let (cache, clock) = cache();
        let url = "https://docs.client.example/oauth/client.json";
        assert_eq!(cache.get(url), None);
        cache.insert(url, doc("Client"));
        clock.advance(DOCUMENT_TTL - Duration::from_secs(1));
        assert_eq!(cache.get(url), Some(doc("Client")));
        clock.advance(Duration::from_secs(1));
        assert_eq!(cache.get(url), None);
        assert!(cache.is_empty(), "an expired entry is dropped on read");
    }

    #[test]
    fn a_refetch_restarts_the_five_minutes() {
        let (cache, clock) = cache();
        let url = "https://docs.client.example/oauth/client.json";
        cache.insert(url, doc("Old"));
        clock.advance(DOCUMENT_TTL - Duration::from_secs(1));
        cache.insert(url, doc("New"));
        clock.advance(Duration::from_secs(2));
        assert_eq!(cache.get(url), Some(doc("New")));
    }

    #[test]
    fn a_full_cache_evicts_the_oldest_document_and_keeps_the_rest() {
        let (cache, clock) = cache();
        for i in 0..MAX_CACHED_DOCUMENTS {
            cache.insert(&format!("https://docs.client.example/{i}.json"), doc("C"));
            clock.advance(Duration::from_millis(1));
        }
        assert_eq!(cache.len(), MAX_CACHED_DOCUMENTS);
        cache.insert("https://docs.client.example/new.json", doc("New"));
        assert_eq!(cache.len(), MAX_CACHED_DOCUMENTS);
        assert_eq!(cache.get("https://docs.client.example/0.json"), None);
        assert!(cache.get("https://docs.client.example/1.json").is_some());
        assert!(cache.get("https://docs.client.example/new.json").is_some());
        // Refreshing a document already held evicts nothing.
        cache.insert("https://docs.client.example/1.json", doc("C"));
        assert_eq!(cache.len(), MAX_CACHED_DOCUMENTS);
    }

    #[test]
    fn a_full_cache_drops_expired_documents_before_live_ones() {
        let (cache, clock) = cache();
        for i in 0..MAX_CACHED_DOCUMENTS - 1 {
            cache.insert(&format!("https://docs.client.example/{i}.json"), doc("C"));
        }
        clock.advance(DOCUMENT_TTL);
        cache.insert("https://docs.client.example/live.json", doc("Live"));
        cache.insert("https://docs.client.example/new.json", doc("New"));
        assert_eq!(cache.len(), 2);
        assert!(cache.get("https://docs.client.example/live.json").is_some());
    }
}
