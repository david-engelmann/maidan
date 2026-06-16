//! HMAC-signed `maidan_session` cookie values.

use std::sync::Arc;

use hmac::{Hmac, Mac};
use maidan_types::SessionId;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::config::ConfigError;

type HmacSha256 = Hmac<Sha256>;

pub fn session_secret_from_env() -> Result<Arc<[u8]>, ConfigError> {
    let raw = std::env::var("MAIDAN_SESSION_SECRET")
        .map_err(|_| ConfigError::Missing("MAIDAN_SESSION_SECRET"))?;
    if raw.len() < 32 {
        return Err(ConfigError::Invalid(
            "MAIDAN_SESSION_SECRET",
            "must be at least 32 bytes".into(),
        ));
    }
    Ok(Arc::from(raw.into_bytes()))
}

pub fn sign_session_value(session_id: SessionId, secret: &[u8]) -> String {
    let mac = mac_for_session(session_id.0, secret);
    format!("{}.{}", session_id.0, hex::encode(mac))
}

pub fn verify_session_value(raw: &str, secret: &[u8]) -> Option<SessionId> {
    let (id_str, mac_hex) = raw.split_once('.')?;
    let session_id = Uuid::parse_str(id_str).ok()?;
    let expected = mac_for_session(session_id, secret);
    let actual = hex::decode(mac_hex).ok()?;
    if actual.len() != expected.len() {
        return None;
    }
    if !bool::from(actual.ct_eq(&expected)) {
        return None;
    }
    Some(SessionId(session_id))
}

fn mac_for_session(session_id: Uuid, secret: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret)
        .unwrap_or_else(|_| unreachable!("HMAC-SHA256 accepts any key length"));
    mac.update(session_id.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-session-secret-32-bytes-min!";

    #[test]
    fn signed_cookie_round_trips_and_rejects_tampering() {
        let id = SessionId(Uuid::new_v4());
        let signed = sign_session_value(id, SECRET);
        assert_eq!(verify_session_value(&signed, SECRET), Some(id));

        let mut tampered = signed.clone();
        tampered.pop();
        tampered.push('x');
        assert_eq!(verify_session_value(&tampered, SECRET), None);

        let bare = id.0.to_string();
        assert_eq!(verify_session_value(&bare, SECRET), None);
    }
}
