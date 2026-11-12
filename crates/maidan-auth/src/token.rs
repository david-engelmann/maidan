use std::fmt;

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Plaintext bearer secret shown once at mint time: `maid_`, 256 random bits
/// from the OS as 43 base62 digits, and a 6-digit base62 CRC32 of those digits.
/// The checksum lets a secret scanner tell a real leaked token from a lookalike
/// offline, without asking the server.
#[derive(Clone)]
pub struct TokenSecret(String);

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// 62^43 > 2^256, so 43 digits hold any 32 bytes.
const BODY_DIGITS: usize = 43;
/// 62^6 > 2^32, so 6 digits hold any CRC32.
const CHECKSUM_DIGITS: usize = 6;
const TOKEN_PREFIX: &str = "maid_";

/// `bytes` as a big-endian number in exactly `digits` base62 digits.
fn base62(mut bytes: Vec<u8>, digits: usize) -> String {
    let mut out = vec![b'0'; digits];
    for slot in out.iter_mut().rev() {
        let mut rem: u32 = 0;
        for byte in bytes.iter_mut() {
            let acc = (rem << 8) | u32::from(*byte);
            *byte = (acc / 62) as u8;
            rem = acc % 62;
        }
        *slot = BASE62[rem as usize];
    }
    out.into_iter().map(char::from).collect()
}

/// CRC-32 (IEEE 802.3, the one GitHub's token checksums use).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn checksum(body: &str) -> String {
    base62(
        crc32(body.as_bytes()).to_be_bytes().to_vec(),
        CHECKSUM_DIGITS,
    )
}

/// Whether `candidate` is a bearer secret in the minted shape with a checksum
/// that matches. Offline only: it says nothing about whether the token is live.
pub fn is_checksummed_token(candidate: &str) -> bool {
    let Some(rest) = candidate.strip_prefix(TOKEN_PREFIX) else {
        return false;
    };
    if rest.len() != BODY_DIGITS + CHECKSUM_DIGITS
        || !rest.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return false;
    }
    let (body, sum) = rest.split_at(BODY_DIGITS);
    checksum(body) == sum
}

impl fmt::Debug for TokenSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenSecret([REDACTED])")
    }
}

impl TokenSecret {
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut random = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut random);
        let body = base62(random, BODY_DIGITS);
        let sum = checksum(&body);
        Self(format!("{TOKEN_PREFIX}{body}{sum}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Plaintext cross-organization share-ticket secret shown once at issuance.
/// The distinct prefix prevents it from being mistaken for an ordinary
/// workspace bearer token at either auth boundary.
#[derive(Clone)]
pub struct ShareTicketSecret(String);

impl fmt::Debug for ShareTicketSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShareTicketSecret([REDACTED])")
    }
}

impl ShareTicketSecret {
    pub fn generate() -> Self {
        let a = uuid::Uuid::new_v4().simple().to_string();
        let b = uuid::Uuid::new_v4().simple().to_string();
        Self(format!("maid_share_{a}{b}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Plaintext workspace-invite secret shown once at minting. The `maid_inv_`
/// prefix keeps it from being taken for a bearer token, and the OIDC
/// sign-up route uses it to tell an invite from a sign-up code.
#[derive(Clone)]
pub struct WorkspaceInviteSecret(String);

impl fmt::Debug for WorkspaceInviteSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkspaceInviteSecret([REDACTED])")
    }
}

impl WorkspaceInviteSecret {
    pub fn generate() -> Self {
        let a = uuid::Uuid::new_v4().simple().to_string();
        let b = uuid::Uuid::new_v4().simple().to_string();
        Self(format!(
            "{}{a}{b}",
            maidan_types::WORKSPACE_INVITE_SECRET_PREFIX
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// SHA-256 hex digest of the bearer secret (64 lowercase hex chars).
pub fn hash_secret(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hashes_equal(stored_hash: &str, computed_hash: &str) -> bool {
    if stored_hash.len() != computed_hash.len() {
        return false;
    }
    stored_hash
        .as_bytes()
        .ct_eq(computed_hash.as_bytes())
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_every_secret() {
        let token = TokenSecret::generate();
        let ticket = ShareTicketSecret::generate();
        let invite = WorkspaceInviteSecret::generate();
        let rendered = format!("{token:?} {ticket:?} {invite:?}");
        assert_eq!(
            rendered,
            "TokenSecret([REDACTED]) ShareTicketSecret([REDACTED]) WorkspaceInviteSecret([REDACTED])"
        );
        assert!(!rendered.contains(token.as_str()));
        assert!(!rendered.contains(ticket.as_str()));
        assert!(!rendered.contains(invite.as_str()));
    }

    #[test]
    fn hash_is_64_hex_chars() {
        let h = hash_secret("maid_test");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hashes_equal_is_constant_time_safe() {
        let a = hash_secret("x");
        let b = hash_secret("x");
        let c = hash_secret("y");
        assert!(hashes_equal(&a, &b));
        assert!(!hashes_equal(&a, &c));
    }

    #[test]
    fn a_minted_token_is_prefix_256_bits_of_base62_and_a_checksum_that_verifies() {
        let token = TokenSecret::generate();
        let secret = token.as_str();
        assert!(secret.starts_with("maid_"));
        assert_eq!(secret.len(), "maid_".len() + 43 + 6);
        assert!(secret["maid_".len()..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric()));
        assert!(is_checksummed_token(secret));
        assert_ne!(secret, TokenSecret::generate().as_str());
    }

    #[test]
    fn a_token_with_one_changed_digit_fails_its_checksum() {
        let token = TokenSecret::generate();
        let secret = token.as_str();
        for at in ["maid_".len(), secret.len() - 1] {
            let mut bytes = secret.as_bytes().to_vec();
            bytes[at] = if bytes[at] == b'0' { b'1' } else { b'0' };
            let tampered = String::from_utf8(bytes).unwrap();
            assert!(!is_checksummed_token(&tampered), "{tampered}");
        }
        assert!(!is_checksummed_token(&format!("maid_{}", "0".repeat(64))));
        assert!(!is_checksummed_token("maid_test"));
        assert!(!is_checksummed_token(&secret.replacen("maid_", "mail_", 1)));
    }

    #[test]
    fn crc32_matches_the_ieee_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn base62_is_fixed_width_and_big_endian() {
        assert_eq!(base62(vec![0; 4], 6), "000000");
        assert_eq!(base62(vec![0, 0, 0, 61], 6), "00000z");
        assert_eq!(base62(vec![0, 0, 0, 62], 6), "000010");
        assert_eq!(base62(vec![0xFF; 32], 43).len(), 43);
    }

    #[test]
    fn share_ticket_secret_has_a_distinct_high_entropy_prefix() {
        let secret = ShareTicketSecret::generate();
        assert!(secret.as_str().starts_with("maid_share_"));
        assert_eq!(secret.as_str().len(), "maid_share_".len() + 64);
    }
}
