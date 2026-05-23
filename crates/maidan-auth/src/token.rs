use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Plaintext bearer secret shown once at mint time (`maid_<uuid><uuid>`).
#[derive(Debug, Clone)]
pub struct TokenSecret(String);

impl TokenSecret {
    pub fn generate() -> Self {
        let a = uuid::Uuid::new_v4().simple().to_string();
        let b = uuid::Uuid::new_v4().simple().to_string();
        Self(format!("maid_{a}{b}"))
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
}
