//! Signed workspace-export envelope.
//!
//! The artifact a blank instance verifies without calling the origin. The
//! Ed25519 signature covers every field except `content_sha256` and `signature`
//! (see [`statement_value`]). Token policy is part of the signed statement so
//! it cannot be rewritten after the fact.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

/// Observable `$type` for a signed workspace export. Breaking changes are a
/// new type (`/2`), not a silent reshape of `/1`.
pub const SIGNED_EXPORT_TYPE: &str = "maidan.workspace.export/1";

/// Only algorithm this cluster accepts. Unknown `alg` fails closed.
pub const SIGNED_EXPORT_ALG: &str = "ed25519";

/// Keys that must never appear in an export payload. Presence is a
/// verification failure — tokens die on export; stuffing them back in is
/// tamper, not a feature.
const FORBIDDEN_PAYLOAD_KEYS: &[&str] = &[
    "api_token",
    "api_tokens",
    "client_secret",
    "encryption_key",
    "fsm_hook_secret",
    "oidc_client_secret",
    "password",
    "peer_secret",
    "private_key",
    "secret",
    "session_secret",
    "slash_secret",
    "token_hash",
    "token_secret",
    "tokens",
    "vapid_private_key",
    "webhook_secret",
];

/// What happens to credentials when a workspace is exported.
///
/// The only policy is [`TokenPolicy::TokensDieOnExport`]: API tokens,
/// webhook/slash/OIDC secrets, and at-rest keys are omitted from the
/// bundle. After import the operator mints new tokens on the destination.
/// Continuity is unsafe (hashed secrets plus AEAD keys do not travel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TokenPolicy {
    TokensDieOnExport,
}

impl TokenPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TokensDieOnExport => "tokens_die_on_export",
        }
    }
}

/// Self-contained signed export a stranger can verify offline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedExport {
    #[serde(rename = "$type")]
    pub type_id: String,
    pub alg: String,
    pub token_policy: TokenPolicy,
    pub public_key: String,
    pub signed_at: chrono::DateTime<chrono::Utc>,
    pub payload: Value,
    pub content_sha256: String,
    pub signature: String,
}

#[derive(Debug, Error)]
pub enum SignedExportError {
    #[error("signed export JSON is not canonicalizable: {0}")]
    Json(String),
    #[error("export payload contains a forbidden secret field: {0}")]
    ForbiddenField(String),
    #[error("invalid hex encoding")]
    InvalidHex,
}

/// Compact JSON with object keys sorted lexicographically. Used as the
/// preimage for `content_sha256` so two semantically equal statements
/// hash the same regardless of serde field order.
pub fn canonical_json(value: &Value) -> Result<Vec<u8>, SignedExportError> {
    let mut out = Vec::new();
    write_canonical(&mut out, value)?;
    Ok(out)
}

fn write_canonical(out: &mut Vec<u8>, value: &Value) -> Result<(), SignedExportError> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => {
            let encoded =
                serde_json::to_string(s).map_err(|e| SignedExportError::Json(e.to_string()))?;
            out.extend_from_slice(encoded.as_bytes());
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(out, item)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                let key_json = serde_json::to_string(key)
                    .map_err(|e| SignedExportError::Json(e.to_string()))?;
                out.extend_from_slice(key_json.as_bytes());
                out.push(b':');
                write_canonical(out, &map[key.as_str()])?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// Fields covered by the signature: everything except `content_sha256`
/// and `signature`.
pub fn statement_value(export: &SignedExport) -> Result<Value, SignedExportError> {
    let mut map = Map::new();
    map.insert("$type".into(), Value::String(export.type_id.clone()));
    map.insert("alg".into(), Value::String(export.alg.clone()));
    map.insert(
        "token_policy".into(),
        serde_json::to_value(export.token_policy)
            .map_err(|e| SignedExportError::Json(e.to_string()))?,
    );
    map.insert(
        "public_key".into(),
        Value::String(export.public_key.clone()),
    );
    map.insert(
        "signed_at".into(),
        serde_json::to_value(export.signed_at)
            .map_err(|e| SignedExportError::Json(e.to_string()))?,
    );
    map.insert("payload".into(), export.payload.clone());
    Ok(Value::Object(map))
}

/// Walk `payload` and fail if any object key is a credential field.
pub fn reject_export_secrets(payload: &Value) -> Result<(), SignedExportError> {
    walk_reject(payload)
}

fn walk_reject(value: &Value) -> Result<(), SignedExportError> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if FORBIDDEN_PAYLOAD_KEYS.contains(&key.as_str()) {
                    return Err(SignedExportError::ForbiddenField(key.clone()));
                }
                walk_reject(child)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                walk_reject(item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn hex_decode(s: &str) -> Result<Vec<u8>, SignedExportError> {
    let trimmed = s.trim();
    if !trimmed.len().is_multiple_of(2) || trimmed.is_empty() {
        return Err(SignedExportError::InvalidHex);
    }
    if !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SignedExportError::InvalidHex);
    }
    let mut out = Vec::with_capacity(trimmed.len() / 2);
    let bytes = trimmed.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, SignedExportError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(SignedExportError::InvalidHex),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn sample_payload() -> Value {
        json!({
            "format_version": 1,
            "workspace": {"name": "room", "id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"},
            "members": [],
            "z_last": true,
            "a_first": 1
        })
    }

    fn sample_export() -> SignedExport {
        SignedExport {
            type_id: SIGNED_EXPORT_TYPE.into(),
            alg: SIGNED_EXPORT_ALG.into(),
            token_policy: TokenPolicy::TokensDieOnExport,
            public_key: "aa".repeat(32),
            signed_at: Utc.with_ymd_and_hms(2026, 9, 14, 0, 0, 0).unwrap(),
            payload: sample_payload(),
            content_sha256: String::new(),
            signature: String::new(),
        }
    }

    #[test]
    fn canonical_json_sorts_object_keys() {
        let left = json!({"b": 1, "a": 2});
        let right = json!({"a": 2, "b": 1});
        assert_eq!(
            canonical_json(&left).unwrap(),
            canonical_json(&right).unwrap()
        );
        assert_eq!(
            canonical_json(&left).unwrap(),
            br#"{"a":2,"b":1}"#.as_slice()
        );
    }

    #[test]
    fn statement_omits_hash_and_signature() {
        let mut export = sample_export();
        export.content_sha256 = "deadbeef".into();
        export.signature = "cafe".into();
        let stmt = statement_value(&export).unwrap();
        let obj = stmt.as_object().unwrap();
        assert!(!obj.contains_key("content_sha256"));
        assert!(!obj.contains_key("signature"));
        assert_eq!(obj["$type"], SIGNED_EXPORT_TYPE);
        assert_eq!(obj["token_policy"], "tokens_die_on_export");
        assert_eq!(obj["payload"]["workspace"]["name"], "room");
    }

    #[test]
    fn reject_export_secrets_flags_token_fields() {
        let leaked = json!({"members": [{"handle": "a", "token_hash": "abc"}]});
        let err = reject_export_secrets(&leaked).unwrap_err();
        assert!(matches!(err, SignedExportError::ForbiddenField(k) if k == "token_hash"));
        reject_export_secrets(&sample_payload()).unwrap();
    }

    #[test]
    fn token_policy_wire_is_tokens_die_on_export() {
        let v = serde_json::to_value(TokenPolicy::TokensDieOnExport).unwrap();
        assert_eq!(v, json!("tokens_die_on_export"));
        let back: TokenPolicy = serde_json::from_value(v).unwrap();
        assert_eq!(back, TokenPolicy::TokensDieOnExport);
        assert!(serde_json::from_value::<TokenPolicy>(json!("survive")).is_err());
    }

    #[test]
    fn hex_round_trip() {
        let bytes = [0x00, 0xab, 0xff];
        assert_eq!(hex_encode(&bytes), "00abff");
        assert_eq!(hex_decode("00abff").unwrap(), bytes);
        assert_eq!(hex_decode("00ABFF").unwrap(), bytes);
        assert!(hex_decode("0").is_err());
        assert!(hex_decode("zz").is_err());
    }
}
