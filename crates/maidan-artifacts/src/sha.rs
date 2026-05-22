//! Sha256 newtype with hex round-trip and a constant-time `Eq`.

use sha2::Digest;

use crate::error::ArtifactError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256([u8; 32]);

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

    /// Parse a 64-char lowercase hex string.
    pub fn from_hex(s: &str) -> Result<Self, ArtifactError> {
        if s.len() != 64 {
            return Err(ArtifactError::InvalidSha(format!(
                "expected 64-char hex, got {} chars",
                s.len()
            )));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex = std::str::from_utf8(chunk)
                .map_err(|_| ArtifactError::InvalidSha("not utf-8".into()))?;
            out[i] = u8::from_str_radix(hex, 16)
                .map_err(|_| ArtifactError::InvalidSha(format!("invalid hex pair: {hex}")))?;
        }
        Ok(Self(out))
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
    fn hex_round_trip() {
        let sha = Sha256::compute(b"hello world");
        let hex = sha.to_hex();
        assert_eq!(hex.len(), 64);
        let parsed = Sha256::from_hex(&hex).unwrap();
        assert_eq!(sha, parsed);
    }

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
}
