//! Crypto-shredding for the words of a message.
//!
//! The event log is append-only and hash-chained: a row can never be edited to
//! forget something without breaking every hash after it. So the words of a
//! `message_posted` or `message_edited` event (`body`, `metadata` and
//! `content`) are encrypted **before** the event is hashed, under a key that
//! belongs to that one message. Withdrawing the message destroys the key. The
//! ciphertext stays in the log, the hash chain still covers exactly the bytes
//! it always covered, and nobody — admin, backup reader of a later dump, or
//! federation peer — can read the words again.
//!
//! - **Data keys** ([`ContentKey`]): one random 256-bit key per message
//!   ([`content_subject`]); a message's posted and edited events share it.
//! - **Sealing** ([`seal_payload`]): XChaCha20-Poly1305 with a random 192-bit
//!   nonce. The associated data binds the ciphertext to its message id.
//! - **Key wrapping** ([`ContentKeyring`]): data keys are stored wrapped by a
//!   server key-encryption key (KEK), the same AEAD, bound to the subject id.
//!   Rotation adds a new primary and keeps the old keys for unwrapping.
//!
//! A payload read back without its key keeps its `sealed` block and an empty
//! body. That is the redacted state, and it is also exactly the bytes the hash
//! covers, so a shredded event still verifies.

use std::fmt;

use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    XChaCha20Poly1305, XNonce,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::events::EventKind;
use crate::ids::{MessageId, PeerId};
use crate::signed_export::{canonical_json, hex_encode};

/// The only sealing algorithm. A different one is a new name, never a reshape.
pub const SEALED_ALG: &str = "xchacha20poly1305";

/// Message fields that carry words. Everything else about a message (id,
/// thread, author, timestamps) stays in the clear.
pub const SEALED_FIELDS: [&str; 3] = ["body", "metadata", "content"];

const NONCE_LEN: usize = 24;
const SEAL_DOMAIN: &[u8] = b"maidan.content.seal/1";
const WRAP_DOMAIN: &[u8] = b"maidan.content-key.wrap/1";
const KEK_ID_DOMAIN: &[u8] = b"maidan.content-kek.id/1";
const INSECURE_DEV_KEK_DOMAIN: &[u8] = b"maidan.content-kek.insecure-dev/1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SealError {
    #[error("event payload has no message to seal")]
    NoMessage,
    #[error("sealed block is malformed")]
    Malformed,
    #[error("unsupported sealing algorithm `{0}`")]
    UnsupportedAlg(String),
    #[error("sealed content failed authentication")]
    Authentication,
    #[error("content key is wrapped by unknown key-encryption key `{0}`")]
    UnknownKek(String),
    #[error("key must be 32 bytes, as base64 or 64 hex characters")]
    InvalidKey,
}

/// A message's data key. Zeroed on drop; never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct ContentKey(Zeroizing<[u8; 32]>);

impl ContentKey {
    pub fn generate() -> Self {
        Self(Zeroizing::new(
            XChaCha20Poly1305::generate_key(&mut OsRng).into(),
        ))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(self.0.as_ref().into())
    }
}

impl fmt::Debug for ContentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ContentKey(..)")
    }
}

impl Serialize for ContentKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(self.0.as_ref()))
    }
}

impl<'de> Deserialize<'de> for ContentKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = Zeroizing::new(String::deserialize(deserializer)?);
        let bytes = Zeroizing::new(
            STANDARD
                .decode(encoded.as_bytes())
                .map_err(serde::de::Error::custom)?,
        );
        let key: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| serde::de::Error::custom("content key must be 32 bytes"))?;
        Ok(Self::from_bytes(key))
    }
}

/// The encrypted words of one message event, as the log stores them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SealedContent {
    /// Always [`SEALED_ALG`].
    pub alg: String,
    /// Base64, 24 bytes.
    pub nonce: String,
    /// Base64 ciphertext and tag of the canonical JSON of the sealed fields.
    pub ciphertext: String,
}

/// What [`open_payload`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Openness {
    /// Nothing in this payload is sealed.
    Plain,
    /// The words were sealed and are now restored.
    Opened,
    /// The words are sealed and their key is gone. The payload is unchanged.
    Shredded,
}

/// Whether events of this kind carry sealed words.
pub fn is_sealed_kind(kind: EventKind) -> bool {
    matches!(kind, EventKind::MessagePosted | EventKind::MessageEdited)
}

/// The key subject for a message: its own id when written here, and a
/// name-based id under the peer's namespace when it arrived by federation.
/// A peer can therefore only ever destroy keys for messages it sent.
pub fn content_subject(message_id: MessageId, origin: Option<PeerId>) -> uuid::Uuid {
    match origin {
        None => message_id.0,
        Some(peer) => uuid::Uuid::new_v5(&peer.0, message_id.0.as_bytes()),
    }
}

/// The message id a message-content payload names.
pub fn payload_message_id(payload: &Value) -> Option<MessageId> {
    payload
        .get("message")?
        .get("id")?
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(MessageId)
}

fn seal_aad(message_id: MessageId) -> Vec<u8> {
    let mut aad = SEAL_DOMAIN.to_vec();
    aad.push(0);
    aad.extend_from_slice(message_id.0.as_bytes());
    aad
}

/// Move the words of a `message_posted`/`message_edited` payload into a
/// `sealed` block encrypted under `key`. The message keeps an empty `body`, so
/// the payload still parses as an event.
pub fn seal_payload(payload: &mut Value, key: &ContentKey) -> Result<MessageId, SealError> {
    let message_id = payload_message_id(payload).ok_or(SealError::NoMessage)?;
    let message = payload
        .get_mut("message")
        .and_then(Value::as_object_mut)
        .ok_or(SealError::NoMessage)?;
    let mut words = Map::new();
    for field in SEALED_FIELDS {
        if let Some(value) = message.remove(field) {
            words.insert(field.to_string(), value);
        }
    }
    message.insert("body".into(), Value::String(String::new()));
    let plaintext =
        Zeroizing::new(canonical_json(&Value::Object(words)).map_err(|_| SealError::Malformed)?);
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = key
        .cipher()
        .encrypt(
            &nonce,
            Payload {
                msg: &plaintext,
                aad: &seal_aad(message_id),
            },
        )
        .map_err(|_| SealError::Authentication)?;
    let sealed = SealedContent {
        alg: SEALED_ALG.to_string(),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    };
    let object = payload.as_object_mut().ok_or(SealError::NoMessage)?;
    object.insert(
        "sealed".into(),
        serde_json::to_value(sealed).map_err(|_| SealError::Malformed)?,
    );
    Ok(message_id)
}

/// Restore the words of a sealed payload with `key`. Without a key the payload
/// is left as it is — sealed, with an empty body — and reported
/// [`Openness::Shredded`]. A ciphertext that fails authentication is an error:
/// it was altered, or the key is not this message's.
pub fn open_payload(payload: &mut Value, key: Option<&ContentKey>) -> Result<Openness, SealError> {
    let Some(sealed) = payload.get("sealed") else {
        return Ok(Openness::Plain);
    };
    let Some(key) = key else {
        return Ok(Openness::Shredded);
    };
    let sealed: SealedContent =
        serde_json::from_value(sealed.clone()).map_err(|_| SealError::Malformed)?;
    if sealed.alg != SEALED_ALG {
        return Err(SealError::UnsupportedAlg(sealed.alg));
    }
    let message_id = payload_message_id(payload).ok_or(SealError::NoMessage)?;
    let nonce = STANDARD
        .decode(&sealed.nonce)
        .map_err(|_| SealError::Malformed)?;
    if nonce.len() != NONCE_LEN {
        return Err(SealError::Malformed);
    }
    let ciphertext = STANDARD
        .decode(&sealed.ciphertext)
        .map_err(|_| SealError::Malformed)?;
    let plaintext = Zeroizing::new(
        key.cipher()
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &seal_aad(message_id),
                },
            )
            .map_err(|_| SealError::Authentication)?,
    );
    let Value::Object(words) =
        serde_json::from_slice::<Value>(&plaintext).map_err(|_| SealError::Malformed)?
    else {
        return Err(SealError::Malformed);
    };
    let object = payload.as_object_mut().ok_or(SealError::NoMessage)?;
    object.remove("sealed");
    let message = object
        .get_mut("message")
        .and_then(Value::as_object_mut)
        .ok_or(SealError::NoMessage)?;
    for (field, value) in words {
        if SEALED_FIELDS.contains(&field.as_str()) {
            message.insert(field, value);
        }
    }
    Ok(Openness::Opened)
}

/// Parse a 32-byte key given as base64 or 64 hex characters.
pub fn parse_key_32(raw: &str) -> Result<[u8; 32], SealError> {
    let trimmed = raw.trim();
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut key = [0u8; 32];
        for (slot, pair) in key.iter_mut().zip(trimmed.as_bytes().chunks(2)) {
            let pair = std::str::from_utf8(pair).map_err(|_| SealError::InvalidKey)?;
            *slot = u8::from_str_radix(pair, 16).map_err(|_| SealError::InvalidKey)?;
        }
        return Ok(key);
    }
    let bytes = Zeroizing::new(
        STANDARD
            .decode(trimmed)
            .map_err(|_| SealError::InvalidKey)?,
    );
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| SealError::InvalidKey)
}

struct Kek {
    id: String,
    key: Zeroizing<[u8; 32]>,
}

impl Kek {
    fn new(key: [u8; 32]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(KEK_ID_DOMAIN);
        hasher.update(key);
        Self {
            id: hex_encode(&hasher.finalize()[..8]),
            key: Zeroizing::new(key),
        }
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(self.key.as_ref().into())
    }
}

/// A data key as stored: which KEK wrapped it, and the wrapped bytes
/// (nonce followed by ciphertext and tag).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedKey {
    pub kek_id: String,
    pub blob: Vec<u8>,
}

/// The server's key-encryption keys: one primary that wraps every new data
/// key, and the previous ones, kept only to unwrap keys not yet rewrapped.
/// A KEK is named by a fingerprint of itself, so configuration lists keys and
/// never ids.
pub struct ContentKeyring {
    primary: Kek,
    previous: Vec<Kek>,
    insecure_dev: bool,
}

impl ContentKeyring {
    pub fn new(primary: [u8; 32], previous: Vec<[u8; 32]>) -> Self {
        Self {
            primary: Kek::new(primary),
            previous: previous.into_iter().map(Kek::new).collect(),
            insecure_dev: false,
        }
    }

    /// A fixed, public KEK for development and tests. Shredding still works —
    /// it destroys the wrapped key — but a database copy taken before a shred
    /// can be unwrapped by anyone. Production refuses to start with it.
    pub fn insecure_dev() -> Self {
        let key: [u8; 32] = Sha256::digest(INSECURE_DEV_KEK_DOMAIN).into();
        Self {
            primary: Kek::new(key),
            previous: Vec::new(),
            insecure_dev: true,
        }
    }

    pub fn is_insecure_dev(&self) -> bool {
        self.insecure_dev
    }

    /// Fingerprint of the primary KEK, as stored beside each key it wraps.
    pub fn primary_id(&self) -> &str {
        &self.primary.id
    }

    fn wrap_aad(subject: uuid::Uuid) -> Vec<u8> {
        let mut aad = WRAP_DOMAIN.to_vec();
        aad.push(0);
        aad.extend_from_slice(subject.as_bytes());
        aad
    }

    /// Wrap `key` for `subject` under the primary KEK.
    pub fn wrap(&self, subject: uuid::Uuid, key: &ContentKey) -> Result<WrappedKey, SealError> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = self
            .primary
            .cipher()
            .encrypt(
                &nonce,
                Payload {
                    msg: key.as_bytes(),
                    aad: &Self::wrap_aad(subject),
                },
            )
            .map_err(|_| SealError::Authentication)?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ciphertext);
        Ok(WrappedKey {
            kek_id: self.primary.id.clone(),
            blob,
        })
    }

    /// Unwrap a stored key. The KEK is found by its fingerprint; one this
    /// keyring does not hold is an error, never a shredded key — a missing
    /// configuration entry must not read as deliberate destruction.
    pub fn unwrap(
        &self,
        subject: uuid::Uuid,
        wrapped: &WrappedKey,
    ) -> Result<ContentKey, SealError> {
        let kek = std::iter::once(&self.primary)
            .chain(&self.previous)
            .find(|kek| kek.id == wrapped.kek_id)
            .ok_or_else(|| SealError::UnknownKek(wrapped.kek_id.clone()))?;
        if wrapped.blob.len() <= NONCE_LEN {
            return Err(SealError::Malformed);
        }
        let (nonce, ciphertext) = wrapped.blob.split_at(NONCE_LEN);
        let bytes = Zeroizing::new(
            kek.cipher()
                .decrypt(
                    XNonce::from_slice(nonce),
                    Payload {
                        msg: ciphertext,
                        aad: &Self::wrap_aad(subject),
                    },
                )
                .map_err(|_| SealError::Authentication)?,
        );
        let key: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| SealError::Malformed)?;
        Ok(ContentKey::from_bytes(key))
    }
}

impl fmt::Debug for ContentKeyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContentKeyring")
            .field("primary", &self.primary.id)
            .field(
                "previous",
                &self.previous.iter().map(|k| &k.id).collect::<Vec<_>>(),
            )
            .field("insecure_dev", &self.insecure_dev)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn posted(message_id: uuid::Uuid) -> Value {
        json!({
            "kind": "message_posted",
            "occurred_at": "2026-09-28T12:00:00Z",
            "workspace_id": uuid::Uuid::nil(),
            "channel_id": uuid::Uuid::nil(),
            "thread_id": uuid::Uuid::nil(),
            "message": {
                "id": message_id,
                "thread_id": uuid::Uuid::nil(),
                "author_id": uuid::Uuid::nil(),
                "body": "the launch code is 0000",
                "metadata": {"n": 1},
                "content": [{"type": "text", "text": "the launch code is 0000"}],
                "posted_at": "2026-09-28T12:00:00Z",
                "edited_at": null,
                "tombstoned_at": null
            }
        })
    }

    #[test]
    fn seal_then_open_restores_every_word() {
        let id = uuid::Uuid::now_v7();
        let original = posted(id);
        let mut payload = original.clone();
        let key = ContentKey::generate();
        assert_eq!(seal_payload(&mut payload, &key).unwrap(), MessageId(id));
        let text = payload.to_string();
        assert!(
            !text.contains("launch code"),
            "sealed payload leaks: {text}"
        );
        assert_eq!(payload["message"]["body"], "");
        assert!(payload["message"].get("metadata").is_none());
        assert!(payload["message"].get("content").is_none());
        assert_eq!(payload["sealed"]["alg"], SEALED_ALG);
        // A sealed payload still parses as an event, with no words in it.
        let event: crate::Event = serde_json::from_value(payload.clone()).unwrap();
        match event {
            crate::Event::MessagePosted {
                message, sealed, ..
            } => {
                assert_eq!(message.body, "");
                assert!(sealed.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            open_payload(&mut payload, Some(&key)).unwrap(),
            Openness::Opened
        );
        assert_eq!(payload, original);
    }

    #[test]
    fn without_the_key_the_payload_stays_sealed() {
        let mut payload = posted(uuid::Uuid::now_v7());
        seal_payload(&mut payload, &ContentKey::generate()).unwrap();
        let sealed = payload.clone();
        assert_eq!(
            open_payload(&mut payload, None).unwrap(),
            Openness::Shredded
        );
        assert_eq!(payload, sealed);
    }

    #[test]
    fn another_key_or_another_message_fails_authentication() {
        let key = ContentKey::generate();
        let mut payload = posted(uuid::Uuid::now_v7());
        seal_payload(&mut payload, &key).unwrap();
        let mut wrong_key = payload.clone();
        assert_eq!(
            open_payload(&mut wrong_key, Some(&ContentKey::generate())),
            Err(SealError::Authentication)
        );
        // The ciphertext is bound to its message id.
        let mut moved = payload.clone();
        moved["message"]["id"] = json!(uuid::Uuid::now_v7());
        assert_eq!(
            open_payload(&mut moved, Some(&key)),
            Err(SealError::Authentication)
        );
        let mut tampered = payload;
        tampered["sealed"]["ciphertext"] = json!(STANDARD.encode([0u8; 40]));
        assert_eq!(
            open_payload(&mut tampered, Some(&key)),
            Err(SealError::Authentication)
        );
    }

    #[test]
    fn an_unsealed_payload_is_plain() {
        let mut payload = posted(uuid::Uuid::now_v7());
        let before = payload.clone();
        assert_eq!(
            open_payload(&mut payload, Some(&ContentKey::generate())).unwrap(),
            Openness::Plain
        );
        assert_eq!(payload, before);
    }

    #[test]
    fn two_seals_of_the_same_words_differ() {
        let key = ContentKey::generate();
        let id = uuid::Uuid::now_v7();
        let (mut a, mut b) = (posted(id), posted(id));
        seal_payload(&mut a, &key).unwrap();
        seal_payload(&mut b, &key).unwrap();
        assert_ne!(a["sealed"]["nonce"], b["sealed"]["nonce"]);
        assert_ne!(a["sealed"]["ciphertext"], b["sealed"]["ciphertext"]);
    }

    #[test]
    fn a_federated_subject_never_equals_a_local_one() {
        let message = MessageId(uuid::Uuid::now_v7());
        let peer = PeerId(uuid::Uuid::now_v7());
        let other = PeerId(uuid::Uuid::now_v7());
        assert_eq!(content_subject(message, None), message.0);
        let federated = content_subject(message, Some(peer));
        assert_ne!(federated, message.0);
        assert_eq!(federated.get_version_num(), 5);
        assert_ne!(federated, content_subject(message, Some(other)));
        assert_eq!(federated, content_subject(message, Some(peer)));
    }

    #[test]
    fn wrap_round_trips_and_is_bound_to_its_subject() {
        let keyring = ContentKeyring::new([7; 32], Vec::new());
        let key = ContentKey::generate();
        let subject = uuid::Uuid::now_v7();
        let wrapped = keyring.wrap(subject, &key).unwrap();
        assert_eq!(wrapped.kek_id, keyring.primary_id());
        assert_eq!(keyring.unwrap(subject, &wrapped).unwrap(), key);
        assert_eq!(
            keyring.unwrap(uuid::Uuid::now_v7(), &wrapped),
            Err(SealError::Authentication)
        );
    }

    #[test]
    fn rotation_unwraps_old_keys_and_wraps_new_ones_under_the_primary() {
        let old = ContentKeyring::new([1; 32], Vec::new());
        let subject = uuid::Uuid::now_v7();
        let key = ContentKey::generate();
        let wrapped = old.wrap(subject, &key).unwrap();
        let rotated = ContentKeyring::new([2; 32], vec![[1; 32]]);
        assert_ne!(rotated.primary_id(), old.primary_id());
        assert_eq!(rotated.unwrap(subject, &wrapped).unwrap(), key);
        assert_eq!(
            rotated.wrap(subject, &key).unwrap().kek_id,
            rotated.primary_id()
        );
        let dropped = ContentKeyring::new([2; 32], Vec::new());
        assert_eq!(
            dropped.unwrap(subject, &wrapped),
            Err(SealError::UnknownKek(old.primary_id().to_string()))
        );
    }

    #[test]
    fn keys_never_print() {
        let key = ContentKey::from_bytes([9; 32]);
        assert_eq!(format!("{key:?}"), "ContentKey(..)");
        let keyring = ContentKeyring::new([9; 32], vec![[8; 32]]);
        let shown = format!("{keyring:?}");
        assert!(!shown.contains("9, 9"), "{shown}");
    }

    #[test]
    fn a_content_key_serializes_as_base64() {
        let key = ContentKey::from_bytes([3; 32]);
        let wire = serde_json::to_value(&key).unwrap();
        assert_eq!(wire, json!(STANDARD.encode([3u8; 32])));
        let back: ContentKey = serde_json::from_value(wire).unwrap();
        assert_eq!(back, key);
        assert!(serde_json::from_value::<ContentKey>(json!("AAAA")).is_err());
    }

    #[test]
    fn keys_parse_from_hex_or_base64() {
        let hex = "ab".repeat(32);
        assert_eq!(parse_key_32(&hex).unwrap(), [0xab; 32]);
        assert_eq!(parse_key_32(&STANDARD.encode([5u8; 32])).unwrap(), [5; 32]);
        assert_eq!(parse_key_32("short"), Err(SealError::InvalidKey));
        assert_eq!(
            parse_key_32(&STANDARD.encode([5u8; 31])),
            Err(SealError::InvalidKey)
        );
    }

    fn stored(payload: Value, content_key: Option<ContentKey>) -> crate::StoredEvent {
        let content_hash = crate::content_hash(&payload).unwrap();
        crate::StoredEvent {
            id: 1,
            lsn: 1,
            kind: EventKind::MessagePosted,
            workspace_id: None,
            channel_id: None,
            thread_id: None,
            payload,
            occurred_at: chrono::Utc::now(),
            prev_hash: crate::genesis_hash(),
            content_hash,
            content_key,
            trace: None,
        }
    }

    #[test]
    fn a_stored_event_serializes_its_key_only_when_keyed() {
        let key = ContentKey::generate();
        let mut payload = posted(uuid::Uuid::now_v7());
        seal_payload(&mut payload, &key).unwrap();
        let event = stored(payload, Some(key.clone()));
        let plain = serde_json::to_value(&event).unwrap();
        assert!(plain.get("content_key").is_none(), "{plain}");
        let keyed = serde_json::to_value(crate::KeyedEvent(event.clone())).unwrap();
        assert_eq!(keyed["content_key"], serde_json::to_value(&key).unwrap());
        // A peer reads the key back and opens the words itself.
        let back: crate::StoredEvent = serde_json::from_value(keyed).unwrap();
        assert_eq!(back.content_key, Some(key));
        assert!(back
            .opened_payload()
            .unwrap()
            .to_string()
            .contains("launch code"));
    }

    #[test]
    fn opening_drops_the_key_and_a_shredded_event_still_verifies() {
        let key = ContentKey::generate();
        let mut payload = posted(uuid::Uuid::now_v7());
        seal_payload(&mut payload, &key).unwrap();
        let mut live = stored(payload.clone(), Some(key));
        assert!(!live.is_shredded());
        live.open().unwrap();
        assert!(live.content_key.is_none());
        assert_eq!(live.payload["message"]["body"], "the launch code is 0000");

        let mut shredded = stored(payload, None);
        assert!(shredded.is_shredded());
        shredded.open().unwrap();
        assert_eq!(shredded.payload["message"]["body"], "");
        let report = crate::verify_chain(&[shredded.link()], &[shredded.payload.clone()]);
        assert!(report.ok, "{report:?}");
        let keyed = serde_json::to_value(crate::KeyedEvent(shredded)).unwrap();
        assert!(keyed.get("content_key").is_none());
    }

    #[test]
    fn the_insecure_dev_keyring_is_marked() {
        assert!(ContentKeyring::insecure_dev().is_insecure_dev());
        assert!(!ContentKeyring::new([1; 32], Vec::new()).is_insecure_dev());
    }
}
