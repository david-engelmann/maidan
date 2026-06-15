//! Federation peer-secret encryption (ChaCha20-Poly1305 AEAD): round-trip,
//! tamper detection, truncation, and the `FEDERATION_ENCRYPTION_KEY` parse
//! matrix.

use base64::{engine::general_purpose::STANDARD, Engine};
use maidan_auth::peer_secret::{encryption_key_from_env, PeerSecretError};
use maidan_auth::{decrypt_peer_secret, encrypt_peer_secret};

const KEY_A: [u8; 32] = [0x11; 32];
const KEY_B: [u8; 32] = [0x22; 32];

#[test]
fn round_trip_recovers_plaintext_and_nonce_randomizes_ciphertext() {
    let plaintext = "maid_peer_outbound_secret_value";
    let ct1 = encrypt_peer_secret(plaintext, &KEY_A).expect("encrypt 1");
    let ct2 = encrypt_peer_secret(plaintext, &KEY_A).expect("encrypt 2");

    // A fresh random nonce per call means identical plaintext encrypts to
    // distinct ciphertext — no deterministic leakage.
    assert_ne!(ct1, ct2, "nonce must randomize the ciphertext");

    assert_eq!(
        decrypt_peer_secret(&ct1, &KEY_A).expect("decrypt 1"),
        plaintext
    );
    assert_eq!(
        decrypt_peer_secret(&ct2, &KEY_A).expect("decrypt 2"),
        plaintext
    );
}

#[test]
fn decrypt_with_wrong_key_fails_authentication() {
    let ct = encrypt_peer_secret("secret", &KEY_A).expect("encrypt");
    let err = decrypt_peer_secret(&ct, &KEY_B).expect_err("wrong key must fail");
    assert!(matches!(err, PeerSecretError::DecryptFailed));
}

#[test]
fn tampering_with_ciphertext_body_is_detected() {
    let ct = encrypt_peer_secret("tamper-me", &KEY_A).expect("encrypt");
    let mut blob = STANDARD.decode(&ct).expect("decode");
    // Flip a bit in the Poly1305-protected ciphertext/tag region (last byte).
    let last = blob.len() - 1;
    blob[last] ^= 0x01;
    let tampered = STANDARD.encode(&blob);
    let err = decrypt_peer_secret(&tampered, &KEY_A).expect_err("tamper must fail");
    assert!(matches!(err, PeerSecretError::DecryptFailed));
}

#[test]
fn tampering_with_the_nonce_is_detected() {
    let ct = encrypt_peer_secret("tamper-the-nonce", &KEY_A).expect("encrypt");
    let mut blob = STANDARD.decode(&ct).expect("decode");
    // Byte 0 is inside the 12-byte prepended nonce.
    blob[0] ^= 0xff;
    let tampered = STANDARD.encode(&blob);
    let err = decrypt_peer_secret(&tampered, &KEY_A).expect_err("nonce tamper must fail");
    assert!(matches!(err, PeerSecretError::DecryptFailed));
}

#[test]
fn truncated_blob_is_rejected_as_invalid_ciphertext() {
    // A blob no longer than the 12-byte nonce can't carry a ciphertext+tag.
    let too_short = STANDARD.encode([0u8; 12]);
    let err = decrypt_peer_secret(&too_short, &KEY_A).expect_err("truncated must fail");
    assert!(matches!(err, PeerSecretError::InvalidCiphertext));
}

#[test]
fn non_base64_input_is_rejected_as_invalid_ciphertext() {
    let err = decrypt_peer_secret("!!! not base64 !!!", &KEY_A).expect_err("garbage must fail");
    assert!(matches!(err, PeerSecretError::InvalidCiphertext));
}

/// The `FEDERATION_ENCRYPTION_KEY` parse matrix. All env mutation is confined
/// to this single test so it stays sequential within this test binary.
#[test]
fn encryption_key_from_env_accepts_hex_and_base64_and_rejects_the_rest() {
    const VAR: &str = "FEDERATION_ENCRYPTION_KEY";

    // Missing -> MissingKey.
    std::env::remove_var(VAR);
    assert!(matches!(
        encryption_key_from_env(),
        Err(PeerSecretError::MissingKey)
    ));

    // 64-char hex -> the corresponding 32 bytes.
    std::env::set_var(VAR, "ab".repeat(32));
    assert_eq!(encryption_key_from_env().expect("hex key"), [0xab; 32]);

    // Surrounding whitespace is trimmed.
    std::env::set_var(VAR, format!("  {}  ", "cd".repeat(32)));
    assert_eq!(encryption_key_from_env().expect("trimmed hex"), [0xcd; 32]);

    // Valid base64 of exactly 32 bytes.
    std::env::set_var(VAR, STANDARD.encode(KEY_A));
    assert_eq!(encryption_key_from_env().expect("base64 key"), KEY_A);

    // Base64 that decodes to the wrong length -> InvalidKey.
    std::env::set_var(VAR, STANDARD.encode([0u8; 16]));
    assert!(matches!(
        encryption_key_from_env(),
        Err(PeerSecretError::InvalidKey)
    ));

    // Neither valid hex nor valid base64 -> InvalidKey.
    std::env::set_var(VAR, "not-a-key");
    assert!(matches!(
        encryption_key_from_env(),
        Err(PeerSecretError::InvalidKey)
    ));

    std::env::remove_var(VAR);
}
