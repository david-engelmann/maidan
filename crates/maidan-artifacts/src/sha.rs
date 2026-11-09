//! Sha256 newtype with hex round-trip and a constant-time `Eq`.

use sha2::Digest;

use crate::error::ArtifactError;

#[derive(Debug, Clone, Copy)]
pub struct Sha256([u8; 32]);

impl PartialEq for Sha256 {
    fn eq(&self, other: &Self) -> bool {
        let mut diff = 0u8;
        for (a, b) in self.0.iter().zip(other.0.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

impl Eq for Sha256 {}

impl std::hash::Hash for Sha256 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl Sha256 {
    /// Hash `bytes` and return the digest.
    pub fn compute(bytes: &[u8]) -> Self {
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        Self(hasher.finalize().into())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in self.0 {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    /// Parse a 64-character hex digest. Uppercase and lowercase ASCII nibbles
    /// are accepted; [`Self::to_hex`] is the canonical lowercase form.
    /// `u8::from_str_radix` is not used: it accepts a leading `+`.
    pub fn from_hex(s: &str) -> Result<Self, ArtifactError> {
        if s.len() != 64 {
            return Err(ArtifactError::InvalidSha(format!(
                "expected 64-char hex, got {} chars",
                s.len()
            )));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hi = hex_nibble(chunk[0])?;
            let lo = hex_nibble(chunk[1])?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }
}

fn hex_nibble(b: u8) -> Result<u8, ArtifactError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(ArtifactError::InvalidSha("invalid hex digit".into())),
    }
}

impl std::fmt::Display for Sha256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // sha256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let sha = Sha256::compute(b"");
        assert_eq!(
            sha.to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn from_hex_rejects_short() {
        assert!(Sha256::from_hex("abc").is_err());
    }

    #[test]
    fn from_hex_rejects_non_hex() {
        let bogus: String = "z".repeat(64);
        assert!(Sha256::from_hex(&bogus).is_err());
    }

    #[test]
    fn from_hex_accepts_uppercase_and_canonicalizes() {
        let sha = Sha256::compute(b"");
        let upper = sha.to_hex().to_ascii_uppercase();
        let parsed = Sha256::from_hex(&upper).unwrap();
        assert_eq!(parsed, sha);
        assert_eq!(parsed.to_hex(), sha.to_hex());
        assert!(parsed.to_hex().chars().all(|c| !c.is_ascii_uppercase()));
    }

    #[test]
    fn eq_and_hash_agree() {
        use std::collections::HashSet;
        use std::hash::{Hash, Hasher};

        let sha = Sha256::compute(b"hello world");
        let again = Sha256::from_hex(&sha.to_hex()).unwrap();
        assert_eq!(sha, again);
        let mut flipped = sha.as_bytes().to_owned();
        flipped[0] ^= 0x01;
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&flipped);
        let other = Sha256(bytes);
        assert_ne!(sha, other);

        fn digest<T: Hash>(value: &T) -> u64 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        }
        assert_eq!(digest(&sha), digest(&again));
        assert_ne!(digest(&sha), digest(&other));
        let mut set = HashSet::new();
        set.insert(sha);
        assert!(set.contains(&again));
        assert!(!set.contains(&other));
    }
}
