//! Property-based tests for the artifact store and sha primitive.

use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, LocalFsStore, Sha256};
use proptest::prelude::*;

proptest! {
    /// Sha::compute is deterministic across invocations.
    #[test]
    fn sha_compute_is_deterministic(payload: Vec<u8>) {
        let a = Sha256::compute(&payload);
        let b = Sha256::compute(&payload);
        prop_assert_eq!(a, b);
    }

    /// Hex round-trip preserves the digest.
    #[test]
    fn sha_hex_round_trip(payload: Vec<u8>) {
        let sha = Sha256::compute(&payload);
        let parsed = Sha256::from_hex(&sha.to_hex()).unwrap();
        prop_assert_eq!(sha, parsed);
    }

    /// Distinct payloads produce distinct shas (with overwhelming probability;
    /// proptest's random inputs are not adversarial collisions).
    #[test]
    fn distinct_payloads_distinct_shas(a: Vec<u8>, b: Vec<u8>) {
        if a != b {
            prop_assert_ne!(Sha256::compute(&a), Sha256::compute(&b));
        }
    }
}

#[tokio::test]
async fn random_payloads_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalFsStore::new(dir.path());
    let mut rng_seed = 0x9E37_79B9_u32; // golden ratio reciprocal; deterministic
    for _ in 0..100 {
        // simple xorshift32 for determinism without a runtime dep
        rng_seed ^= rng_seed << 13;
        rng_seed ^= rng_seed >> 17;
        rng_seed ^= rng_seed << 5;
        let len = (rng_seed as usize) % 4096;
        let payload: Vec<u8> = (0..len)
            .map(|i| ((rng_seed >> (i % 24)) & 0xff) as u8)
            .collect();
        let bytes = Bytes::from(payload.clone());
        let sha = store.put(bytes.clone()).await.expect("put");
        let got = store.get(&sha).await.expect("get");
        assert_eq!(got.as_ref(), payload.as_slice());
    }
}
