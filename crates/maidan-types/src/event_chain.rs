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
//! Cluster 395's hierarchical `maidan://{workspace_id}/channels/…`
//! room scheme ([`crate::RoomUri`]). A handle is an alias, never the
//! authority.

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

/// `claim_next` body: the claimed thread plus a content-addressed pin.
/// `pin` is additive; thread fields stay at the top level (`flatten`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClaimedThread {
    #[serde(flatten)]
    pub thread: crate::models::Thread,
    pub pin: StrongRef,
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

impl ChainBreakReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContentHashMismatch => "content_hash_mismatch",
            Self::PrevHashMismatch => "prev_hash_mismatch",
            Self::MalformedHash => "malformed_hash",
            Self::IdNotIncreasing => "id_not_increasing",
        }
    }
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

/// Pin for a successful `claim_next`. Prefers the `ThreadAssignmentChanged`
/// event (in the hash chain). Falls back to a thread-snapshot pin.
pub fn claim_pin(
    thread: &crate::models::Thread,
    events: &[crate::events::StoredEvent],
) -> Result<StrongRef, EventChainError> {
    if let Some(stored) = events
        .iter()
        .rev()
        .find(|e| e.kind == crate::events::EventKind::ThreadAssignmentChanged)
    {
        return Ok(StrongRef::event(stored.id, stored.content_hash.clone()));
    }
    Ok(StrongRef::thread(thread.id, content_hash_of(thread)?))
}

/// Wrap a claimed thread with its pin.
pub fn claimed_thread(
    thread: crate::models::Thread,
    events: &[crate::events::StoredEvent],
) -> Result<ClaimedThread, EventChainError> {
    Ok(ClaimedThread {
        pin: claim_pin(&thread, events)?,
        thread,
    })
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
/// Rewrite every integral floating-point number in `payload` as an integer, in
/// place, so the value hashes the same before and after a storage round trip.
///
/// # Why this exists
///
/// `maidan_events.payload` is Postgres `jsonb`, which parses each number into
/// `numeric` and re-renders it. Measured against pg17, that preserves the
/// decimal form (`1.0`→`1.0`, `1.10`→`1.10`, `0.1`→`0.1`) but **expands
/// exponent notation**: `1E2`→`100`, `2.5e3`→`2500`.
///
/// That one case breaks the chain. `{"x": 1e2}` parses in memory as an `f64`
/// and renders as `100.0`; jsonb stores `100`, which reads back as an
/// *integer* and renders as `100`. `verify_chain` recomputes the hash from the
/// stored payload, gets a different answer, and reports a tamper on an event
/// nobody touched — permanently, for that workspace. Message `metadata` is
/// arbitrary client JSON and `JSON.stringify` emits exponents above `1e21`, so
/// this is reachable from ordinary use, not only federation.
///
/// Collapsing integral floats to integers removes the ambiguity the round trip
/// introduces: both spellings now hash identically, so it no longer matters
/// which one comes back.
///
/// # What is deliberately left alone
///
/// **Integers are never routed through `f64`** — a `u64` past 2^53 would lose
/// precision, and it already round-trips exactly.
///
/// **Integral floats outside integer range** (`1e30`) keep their shortest form.
/// jsonb expands them to a long decimal, but re-parsing that lands back on the
/// same `f64` and renders the same way, so they already agree.
///
/// **Non-integral values are untouched.** `f64` → shortest decimal → exact
/// `numeric` → decimal → `f64` round-trips, so `1.10` and `0.1` already agree.
///
/// This normalizes the *payload*, not [`canonical_json`]. Changing the
/// canonicaliser would invalidate every stored chain hash and every signed
/// export, with no rebuild path (Cluster 397.8 removed it deliberately).
/// Normalizing the payload changes the hash only of payloads that are
/// currently unverifiable anyway.
pub fn normalize_payload_numbers(payload: &mut Value) {
    match payload {
        Value::Number(n) => {
            let Some(f) = n.as_f64() else {
                return;
            };
            // `as_i64`/`as_u64` succeed for a JSON integer, and those must not
            // be rewritten — this is only about floats that happen to be whole.
            if n.as_i64().is_some() || n.as_u64().is_some() {
                return;
            }
            if !f.is_finite() || f.fract() != 0.0 {
                return;
            }
            if f >= 0.0 && f <= u64::MAX as f64 {
                *n = serde_json::Number::from(f as u64);
            } else if f >= i64::MIN as f64 && f < 0.0 {
                *n = serde_json::Number::from(f as i64);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_payload_numbers),
        Value::Object(map) => map.values_mut().for_each(normalize_payload_numbers),
        _ => {}
    }
}

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
/// Verify a chain one row at a time, holding only the previous link
/// (Cluster 397.8).
///
/// [`verify_chain`] takes whole slices, so the store collected every link *and*
/// a clone of every payload before checking any of them — on a large workspace,
/// gigabytes of resident memory per request, on the lowest read capability.
/// Verification is a fold, not a collect, so this exposes it as one: the caller
/// pages the log and discards each page as it goes.
///
/// Semantics are identical to [`verify_chain`], which now delegates to it.
#[derive(Debug)]
pub struct ChainVerifier {
    genesis: String,
    from_genesis: bool,
    previous: Option<EventLink>,
    checked: u32,
    started: bool,
}

impl Default for ChainVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainVerifier {
    pub fn new() -> Self {
        Self {
            genesis: genesis_hash(),
            from_genesis: true,
            previous: None,
            checked: 0,
            started: false,
        }
    }

    /// Check one row. `Some(report)` is a break — stop paging; the report is
    /// final. `None` means keep going.
    pub fn push(&mut self, link: &EventLink, payload: &Value) -> Option<ChainVerifyReport> {
        // The first row decides whether this chain starts at genesis or at a
        // retention floor, exactly as the slice version reads `links[0]`.
        if !self.started {
            self.from_genesis = link.prev_hash == self.genesis;
            self.started = true;
        }
        let floor_genesis = self.from_genesis && self.checked == 0;
        if let Err(reason) = verify_link(link, payload, self.previous.as_ref(), floor_genesis) {
            return Some(ChainVerifyReport {
                ok: false,
                algorithm: EVENT_CHAIN_ALG.to_string(),
                genesis: self.genesis.clone(),
                checked: self.checked,
                head: self.previous.clone(),
                from_genesis: self.from_genesis,
                break_at: Some(link.id),
                reason: Some(reason),
            });
        }
        self.previous = Some(link.clone());
        self.checked += 1;
        None
    }

    /// The report for a chain that verified all the way to its head.
    pub fn finish(self) -> ChainVerifyReport {
        ChainVerifyReport {
            ok: true,
            algorithm: EVENT_CHAIN_ALG.to_string(),
            genesis: self.genesis,
            checked: self.checked,
            head: self.previous,
            from_genesis: self.from_genesis,
            break_at: None,
            reason: None,
        }
    }
}

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

    let mut verifier = ChainVerifier::new();
    for (link, payload) in links.iter().zip(payloads.iter()) {
        if let Some(report) = verifier.push(link, payload) {
            return report;
        }
    }
    verifier.finish()
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

pub fn is_well_formed_hash(s: &str) -> bool {
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

    /// The window that breaks: serde renders an `f64` with an exponent from
    /// 1e16 up, and Postgres `jsonb` expands that to a plain integer which
    /// still fits `u64` below ~1.8e19 — so it reads back as an integer and
    /// hashes differently from the float that went in.
    #[test]
    fn integral_floats_in_the_jsonb_rewrite_window_become_integers() {
        for (input, expected) in [
            (1e16_f64, 10_000_000_000_000_000u64),
            (5e18, 5_000_000_000_000_000_000),
            (1e2, 100),
        ] {
            let mut v = serde_json::json!({ "n": input });
            normalize_payload_numbers(&mut v);
            assert_eq!(
                v["n"],
                serde_json::json!(expected),
                "{input:e} should normalize to an integer"
            );
        }
    }

    /// Over-reaching is the failure mode worth guarding: normalizing is allowed
    /// to change a number's spelling, never its value.
    #[test]
    fn normalization_leaves_every_other_shape_alone() {
        let original = serde_json::json!({
            "fractional": 0.1,
            "trailing_zero": 1.10,
            "tiny": 1e-7,
            // Past u64 — both sides already route through f64 and agree.
            "huge": 1e30,
            // An integer past 2^53 must never go through f64 or it loses a bit.
            "exact_big_integer": 9007199254740993i64,
            "plain": 3,
            "text": "1e16",
            "null": null,
        });
        let mut v = original.clone();
        normalize_payload_numbers(&mut v);
        assert_eq!(v, original);
    }

    /// The property that actually matters: normalize, and the value hashes the
    /// same whether or not something re-spelled its numbers in between.
    #[test]
    fn a_normalized_payload_hashes_the_same_as_its_respelled_self() {
        let mut in_memory = serde_json::json!({ "a": 1e16, "b": [2e17, { "c": 3e16 }] });
        // What jsonb hands back: the same values, spelled as integers.
        let mut from_storage = serde_json::json!({
            "a": 10_000_000_000_000_000u64,
            "b": [200_000_000_000_000_000u64, { "c": 30_000_000_000_000_000u64 }]
        });
        assert_ne!(
            content_hash(&in_memory).unwrap(),
            content_hash(&from_storage).unwrap(),
            "the two spellings must genuinely differ, or this proves nothing"
        );
        normalize_payload_numbers(&mut in_memory);
        normalize_payload_numbers(&mut from_storage);
        assert_eq!(
            content_hash(&in_memory).unwrap(),
            content_hash(&from_storage).unwrap()
        );
    }

    /// Nested structures are walked, not just the top level.
    #[test]
    fn normalization_reaches_into_arrays_and_objects() {
        let mut v = serde_json::json!({ "a": [{ "b": [[1e16]] }] });
        normalize_payload_numbers(&mut v);
        assert_eq!(
            v["a"][0]["b"][0][0],
            serde_json::json!(10_000_000_000_000_000u64)
        );
    }
}
