//! The token in an `approval_decide` confirmation link.
//!
//! A token is an HMAC of the confirmation row's `nonce` under the server's
//! secret, so the server can hand the same live link back to a model that asks
//! again without keeping the token anywhere: the row stores only the token's
//! SHA-256, and someone who reads the database cannot rebuild a link from it.
//! The token on its own confirms nothing. Confirming also needs the signed-in
//! session of the member the row is bound to, sent from the console page.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Separates this token's MAC from every other use of the same secret
/// (session cookies, `requestState`, subscribe-resume tokens).
const DOMAIN: &[u8] = b"maidan.approval-confirmation.v1\0";

/// The link token for a confirmation `nonce`, hex-encoded.
pub fn token(secret: &[u8], nonce: uuid::Uuid) -> String {
    let mut mac = HmacSha256::new_from_slice(secret)
        .unwrap_or_else(|_| unreachable!("HMAC-SHA256 accepts any key length"));
    mac.update(DOMAIN);
    mac.update(nonce.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// What the store keeps and looks a token up by: its SHA-256, hex-encoded.
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_stable_for_its_nonce_and_differs_by_secret_and_nonce() {
        let nonce = uuid::Uuid::now_v7();
        assert_eq!(token(b"secret", nonce), token(b"secret", nonce));
        assert_ne!(token(b"secret", nonce), token(b"other", nonce));
        assert_ne!(
            token(b"secret", nonce),
            token(b"secret", uuid::Uuid::now_v7())
        );
    }

    #[test]
    fn the_stored_hash_is_not_the_token() {
        let t = token(b"secret", uuid::Uuid::now_v7());
        assert_ne!(token_hash(&t), t);
        assert_eq!(token_hash(&t).len(), 64);
    }
}
