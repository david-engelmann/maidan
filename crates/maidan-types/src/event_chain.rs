//! Hash-chained event log + strong refs (Cluster 392, Wave 3 #32).
//!
//! Every event is accompanied by `{id, lsn, prev_hash, content_hash}`.
//! `lsn` **is** the event-log id (the Cluster 390 room head of this row),
//! not a Postgres WAL [`crate::Lsn`] / `Maidan-Consistency-Token`.
//!
//! **Hashed, not signed.** SHA-256 over Cluster 391 canonical JSON. A
//! federated peer that has seen a prefix (or pulls a workspace page)
//! detects a splice, deletion, or payload rewrite without trusting the
//! host. Authorship of a *wholly fabricated* but internally consistent
//! chain is Cluster 391's signed export, not this module. Not MST/CAR.
//!
//! **Per-workspace.** `maidan_events` is globally sequenced; federation
//! is workspace-scoped, so `prev_hash` links the previous event **in
//! the same workspace** (`ORDER BY id`). Global id gaps (rolled-back
//! inserts, other tenants) are expected.
//!
//! **Retention.** After prune, verify the retained suffix. The oldest
//! remaining row is the floor; it need not chain from genesis. Snapshot
//! catch-up of a pruned prefix is Open Work #33 — out of scope.
//!
//! Strong-ref URIs are `maidan:{event|thread|message}/{id}` pins, **not**
//! the Cluster-#35 `maidan://{room}/…` handle scheme.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ids::{MessageId, ThreadId};
use crate::signed_export::{canonical_json, hex_decode, hex_encode, SignedExportError};

/// Observable `$type` for the chain algorithm. Breaking changes are `/2`.
pub const EVENT_CHAIN_TYPE: &str = "maidan.event-log.chain/1";

/// Only algorithm this cluster accepts.
pub const EVENT_CHAIN_ALG: &str = "sha256";

/// Self-describing hash prefix (distinct from artifact SHA, which is bare hex).
pub const HASH_PREFIX: &str = "sha256:";

/// Domain-separated genesis preimage. First event in a workspace (or an
/// unscoped chain) sets `prev_hash` to [`genesis_hash`].
const GENESIS_DOMAIN: &[u8] = b"maidan.event-log.genesis/1";

/// One log row's chain fields. `lsn` equals `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EventLink {
    pub id: i64,
    pub lsn: i64,
    pub prev_hash: String,
    pub content_hash: String,
}

/// Content-addressed pin. `uri` names the object; `content_hash` is
/// `sha256:<hex>` of its canonical JSON (or of the log event that
/// committed it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StrongRef {
    pub uri: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ChainBreakReason {
    ContentHashMismatch,
    PrevHashMismatch,
    MalformedHash,
    IdNotIncreasing,
}

/// Report from walking a chain. `ok` is the fail-closed bit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChainVerifyReport {
    pub ok: bool,
    pub algorithm: String,
    pub genesis: String,
    pub checked: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<EventLink>,
    pub from_genesis: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<ChainBreakReason>,
}

#[derive(Debug, Error)]
pub enum EventChainError {
    #[error("event chain JSON is not canonicalizable: {0}")]
    Json(String),
    #[error("malformed event-log hash")]
    MalformedHash,
}

impl From<SignedExportError> for EventChainError {
    fn from(err: SignedExportError) -> Self {
        match err {
            SignedExportError::InvalidHex => Self::MalformedHash,
            other => Self::Json(other.to_string()),
        }
    }
}

impl StrongRef {
    pub fn event(id: i64, content_hash: impl Into<String>) -> Self {
        Self {
            uri: event_uri(id),
            content_hash: content_hash.into(),
        }
    }

    pub fn thread(id: ThreadId, content_hash: impl Into<String>) -> Self {
        Self {
            uri: thread_uri(id),
            content_hash: content_hash.into(),
        }
    }

    pub fn message(id: MessageId, content_hash: impl Into<String>) -> Self {
        Self {
            uri: message_uri(id),
            content_hash: content_hash.into(),
        }
    }
}

pub fn event_uri(id: i64) -> String {
    format!("maidan:event/{id}")
}

pub fn thread_uri(id: ThreadId) -> String {
    format!("maidan:thread/{}", id.0)
}

pub fn message_uri(id: MessageId) -> String {
    format!("maidan:message/{}", id.0)
}

/// SHA-256 of the genesis domain, encoded `sha256:<hex>`.
pub fn genesis_hash() -> String {
    encode_digest(&Sha256::digest(GENESIS_DOMAIN))
}

/// Hash canonical JSON of `payload` (the stored Event, not the wrapper).
pub fn content_hash(payload: &Value) -> Result<String, EventChainError> {
    let bytes = canonical_json(payload)?;
    Ok(hash_bytes(&bytes))
}

/// Hash any JSON-serializable value (thread / message pins).
pub fn content_hash_of<T: Serialize>(value: &T) -> Result<String, EventChainError> {
    let payload = serde_json::to_value(value).map_err(|e| EventChainError::Json(e.to_string()))?;
    content_hash(&payload)
}

/// Commitment the next row stores as `prev_hash`.
///
/// `SHA-256(prev_hash || "\n" || content_hash || "\n" || decimal_id)` so a
/// peer binds payload to position. Ids may gap; the chain follows existing
/// rows in `id` order, not `id - 1`.
pub fn chain_hash(prev_hash: &str, content_hash: &str, id: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(b"\n");
    hasher.update(content_hash.as_bytes());
    hasher.update(b"\n");
    hasher.update(id.to_string().as_bytes());
    encode_digest(&hasher.finalize())
}

/// `prev_hash` the next append should write, given the workspace head.
pub fn next_prev_hash(head: Option<&EventLink>) -> String {
    match head {
        None => genesis_hash(),
        Some(link) => chain_hash(&link.prev_hash, &link.content_hash, link.id),
    }
}

/// Build the link for `id` given the previous head and the event payload.
pub fn link_for(
    id: i64,
    payload: &Value,
    previous: Option<&EventLink>,
) -> Result<EventLink, EventChainError> {
    let content_hash = content_hash(payload)?;
    Ok(EventLink {
        id,
        lsn: id,
        prev_hash: next_prev_hash(previous),
        content_hash,
    })
}

/// Check one row: payload matches `content_hash`, `prev_hash` matches the
/// predecessor (or genesis when `previous` is `None` and `from_genesis`).
///
/// When the retained floor is not genesis (`from_genesis == false` and
/// `previous == None`), `prev_hash` is not checked — retention dropped the
/// parent. Content is still checked (fail closed on rewrite).
pub fn verify_link(
    link: &EventLink,
    payload: &Value,
    previous: Option<&EventLink>,
    from_genesis: bool,
) -> Result<(), ChainBreakReason> {
    if !is_well_formed_hash(&link.prev_hash) || !is_well_formed_hash(&link.content_hash) {
        return Err(ChainBreakReason::MalformedHash);
    }
    if let Some(prev) = previous {
        if link.id <= prev.id {
            return Err(ChainBreakReason::IdNotIncreasing);
        }
        let expected = chain_hash(&prev.prev_hash, &prev.content_hash, prev.id);
        if link.prev_hash != expected {
            return Err(ChainBreakReason::PrevHashMismatch);
        }
    } else if from_genesis && link.prev_hash != genesis_hash() {
        return Err(ChainBreakReason::PrevHashMismatch);
    }
    match content_hash(payload) {
        Ok(expected) if expected == link.content_hash => Ok(()),
        Ok(_) => Err(ChainBreakReason::ContentHashMismatch),
        Err(_) => Err(ChainBreakReason::MalformedHash),
    }
}

/// Walk `links` with matching `payloads`. Empty is ok.
pub fn verify_chain(links: &[EventLink], payloads: &[Value]) -> ChainVerifyReport {
    let genesis = genesis_hash();
    if links.len() != payloads.len() {
        return ChainVerifyReport {
            ok: false,
            algorithm: EVENT_CHAIN_ALG.to_string(),
            genesis,
            checked: 0,
            head: None,
            from_genesis: true,
            break_at: links.first().map(|l| l.id),
            reason: Some(ChainBreakReason::MalformedHash),
        };
    }
    if links.is_empty() {
        return ChainVerifyReport {
            ok: true,
            algorithm: EVENT_CHAIN_ALG.to_string(),
            genesis,
            checked: 0,
            head: None,
            from_genesis: true,
            break_at: None,
            reason: None,
        };
    }

    let from_genesis = links[0].prev_hash == genesis;
    let mut previous: Option<&EventLink> = None;
    for (i, (link, payload)) in links.iter().zip(payloads.iter()).enumerate() {
        let floor_genesis = from_genesis && i == 0;
        if let Err(reason) = verify_link(link, payload, previous, floor_genesis) {
            return ChainVerifyReport {
                ok: false,
                algorithm: EVENT_CHAIN_ALG.to_string(),
                genesis,
                checked: i as u32,
                head: previous.cloned(),
                from_genesis,
                break_at: Some(link.id),
                reason: Some(reason),
            };
        }
        previous = Some(link);
    }
    ChainVerifyReport {
        ok: true,
        algorithm: EVENT_CHAIN_ALG.to_string(),
        genesis,
        checked: links.len() as u32,
        head: links.last().cloned(),
        from_genesis,
        break_at: None,
        reason: None,
    }
}

/// Peer-side check: the envelope's stored hashes match its payload and,
/// when `previous` is known, the origin chain continues.
pub fn verify_peer_link(
    link: &EventLink,
    payload: &Value,
    previous: Option<&EventLink>,
) -> Result<(), ChainBreakReason> {
    let from_genesis = previous.is_none() && link.prev_hash == genesis_hash();
    verify_link(link, payload, previous, from_genesis)
}

fn hash_bytes(bytes: &[u8]) -> String {
    encode_digest(&Sha256::digest(bytes))
}

fn encode_digest(digest: &[u8]) -> String {
    format!("{HASH_PREFIX}{}", hex_encode(digest))
}

fn is_well_formed_hash(s: &str) -> bool {
    let Some(hex) = s.strip_prefix(HASH_PREFIX) else {
        return false;
    };
    hex.len() == 64 && hex_decode(hex).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(n: u32) -> Value {
        json!({"kind": "message_posted", "n": n, "z": true, "a": 1})
    }

    #[test]
    fn genesis_is_stable_sha256_of_domain() {
        let expected = {
            let d = Sha256::digest(GENESIS_DOMAIN);
            format!("sha256:{}", hex_encode(&d))
        };
        assert_eq!(genesis_hash(), expected);
        assert!(is_well_formed_hash(&genesis_hash()));
        assert_ne!(genesis_hash(), content_hash(&json!({})).unwrap());
    }

    #[test]
    fn content_hash_ignores_object_key_order() {
        let left = json!({"b": 1, "a": {"y": 2, "x": 3}});
        let right = json!({"a": {"x": 3, "y": 2}, "b": 1});
        assert_eq!(content_hash(&left).unwrap(), content_hash(&right).unwrap());
        let other = json!({"a": 1, "b": 2});
        assert_ne!(content_hash(&left).unwrap(), content_hash(&other).unwrap());
    }

    #[test]
    fn append_then_verify_ok() {
        let p1 = payload(1);
        let p2 = payload(2);
        let e1 = link_for(4, &p1, None).unwrap();
        assert_eq!(e1.id, 4);
        assert_eq!(e1.lsn, 4);
        assert_eq!(e1.prev_hash, genesis_hash());
        let e2 = link_for(9, &p2, Some(&e1)).unwrap();
        assert_eq!(
            e2.prev_hash,
            chain_hash(&e1.prev_hash, &e1.content_hash, e1.id)
        );
        let report = verify_chain(&[e1.clone(), e2.clone()], &[p1, p2]);
        assert!(report.ok, "{report:?}");
        assert_eq!(report.checked, 2);
        assert!(report.from_genesis);
        assert_eq!(report.head.as_ref(), Some(&e2));
        assert_eq!(report.algorithm, EVENT_CHAIN_ALG);
    }

    #[test]
    fn payload_tamper_is_content_hash_mismatch() {
        let p1 = payload(1);
        let e1 = link_for(1, &p1, None).unwrap();
        let tampered = payload(99);
        let report = verify_chain(&[e1], &[tampered]);
        assert!(!report.ok);
        assert_eq!(report.break_at, Some(1));
        assert_eq!(report.reason, Some(ChainBreakReason::ContentHashMismatch));
        assert_eq!(report.checked, 0);
    }

    #[test]
    fn prev_hash_break_is_detected() {
        let p1 = payload(1);
        let p2 = payload(2);
        let e1 = link_for(1, &p1, None).unwrap();
        let mut e2 = link_for(2, &p2, Some(&e1)).unwrap();
        e2.prev_hash = genesis_hash();
        let report = verify_chain(&[e1, e2], &[p1, p2]);
        assert!(!report.ok);
        assert_eq!(report.break_at, Some(2));
        assert_eq!(report.reason, Some(ChainBreakReason::PrevHashMismatch));
        assert_eq!(report.checked, 1);
    }

    #[test]
    fn splice_in_the_middle_breaks_the_successor() {
        let p1 = payload(1);
        let p2 = payload(2);
        let p3 = payload(3);
        let e1 = link_for(1, &p1, None).unwrap();
        let e2 = link_for(2, &p2, Some(&e1)).unwrap();
        let e3 = link_for(3, &p3, Some(&e2)).unwrap();
        // Host drops e2 but leaves e3 pointing at e2's commitment.
        let report = verify_chain(&[e1, e3], &[p1, p3]);
        assert!(!report.ok);
        assert_eq!(report.break_at, Some(3));
        assert_eq!(report.reason, Some(ChainBreakReason::PrevHashMismatch));
    }

    #[test]
    fn retained_suffix_skips_genesis_check() {
        let p1 = payload(1);
        let p2 = payload(2);
        let e1 = link_for(5, &p1, None).unwrap();
        let e2 = link_for(8, &p2, Some(&e1)).unwrap();
        // Simulate prune of e1: e2 is the floor and does not start at genesis.
        let report = verify_chain(std::slice::from_ref(&e2), std::slice::from_ref(&p2));
        assert!(report.ok, "{report:?}");
        assert!(!report.from_genesis);
        assert_eq!(report.checked, 1);
        assert_eq!(report.head.as_ref(), Some(&e2));
    }

    #[test]
    fn empty_chain_is_ok() {
        let report = verify_chain(&[], &[]);
        assert!(report.ok);
        assert_eq!(report.checked, 0);
        assert!(report.head.is_none());
        assert!(report.from_genesis);
    }

    #[test]
    fn malformed_hash_fails_closed() {
        let p = payload(1);
        let mut e = link_for(1, &p, None).unwrap();
        e.content_hash = "deadbeef".into();
        let report = verify_chain(&[e], &[p]);
        assert!(!report.ok);
        assert_eq!(report.reason, Some(ChainBreakReason::MalformedHash));
    }

    #[test]
    fn id_must_increase() {
        let p1 = payload(1);
        let p2 = payload(2);
        let e1 = link_for(5, &p1, None).unwrap();
        let mut e2 = link_for(5, &p2, Some(&e1)).unwrap();
        e2.id = 5;
        e2.lsn = 5;
        let report = verify_chain(&[e1, e2], &[p1, p2]);
        assert!(!report.ok);
        assert_eq!(report.reason, Some(ChainBreakReason::IdNotIncreasing));
    }

    #[test]
    fn peer_verify_accepts_genesis_then_successor() {
        let p1 = payload(1);
        let p2 = payload(2);
        let e1 = link_for(10, &p1, None).unwrap();
        let e2 = link_for(11, &p2, Some(&e1)).unwrap();
        verify_peer_link(&e1, &p1, None).unwrap();
        verify_peer_link(&e2, &p2, Some(&e1)).unwrap();
        // No predecessor: content still checks (catch-up / retained floor);
        // a payload rewrite fails closed.
        verify_peer_link(&e2, &p2, None).unwrap();
        assert_eq!(
            verify_peer_link(&e2, &payload(99), None).unwrap_err(),
            ChainBreakReason::ContentHashMismatch
        );
    }

    #[test]
    fn strong_ref_uris_are_pins_not_room_handles() {
        let hash = content_hash(&json!({"t": 1})).unwrap();
        let event = StrongRef::event(42, hash.clone());
        assert_eq!(event.uri, "maidan:event/42");
        assert_eq!(event.content_hash, hash);
        assert!(!event.uri.starts_with("maidan://"));
        let tid = ThreadId(uuid::Uuid::nil());
        assert_eq!(
            StrongRef::thread(tid, hash.clone()).uri,
            format!("maidan:thread/{}", uuid::Uuid::nil())
        );
        let mid = MessageId(uuid::Uuid::nil());
        assert_eq!(
            StrongRef::message(mid, hash).uri,
            format!("maidan:message/{}", uuid::Uuid::nil())
        );
    }

    #[test]
    fn chain_hash_binds_id_so_reposition_is_a_break() {
        let p = payload(1);
        let e = link_for(1, &p, None).unwrap();
        let moved = EventLink {
            id: 99,
            lsn: 99,
            prev_hash: e.prev_hash.clone(),
            content_hash: e.content_hash.clone(),
        };
        // Same hashes at a different id: the *next* link would not match
        // `chain_hash` of the moved row.
        assert_ne!(
            chain_hash(&e.prev_hash, &e.content_hash, e.id),
            chain_hash(&moved.prev_hash, &moved.content_hash, moved.id)
        );
    }
}
