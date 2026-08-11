//! Secret key rotation (Cluster 189): after the primary key is rotated, a
//! ciphertext made with the *old* key still decrypts via a fallback.

use maidan_auth::{decrypt_peer_secret_rotating, encrypt_peer_secret, init_decrypt_fallback_keys};

const OLD_KEY: [u8; 32] = [0xa1; 32];
const NEW_PRIMARY: [u8; 32] = [0xb2; 32];

#[test]
fn rotated_primary_still_decrypts_old_ciphertext_via_fallback() {
    // Operator rotated: NEW_PRIMARY is now FEDERATION_ENCRYPTION_KEY, OLD_KEY
    // moved into FEDERATION_DECRYPT_KEYS.
    init_decrypt_fallback_keys(vec![OLD_KEY]);

    // A secret encrypted before the rotation (with the old key).
    let old_ct = encrypt_peer_secret("pre-rotation-secret", &OLD_KEY).expect("encrypt old");
    // Decrypt with the new primary: primary fails, the OLD_KEY fallback wins.
    assert_eq!(
        decrypt_peer_secret_rotating(&old_ct, &NEW_PRIMARY).expect("rotating decrypt"),
        "pre-rotation-secret"
    );

    // A secret encrypted after the rotation decrypts on the first try.
    let new_ct = encrypt_peer_secret("post-rotation-secret", &NEW_PRIMARY).expect("encrypt new");
    assert_eq!(
        decrypt_peer_secret_rotating(&new_ct, &NEW_PRIMARY).expect("rotating decrypt new"),
        "post-rotation-secret"
    );

    // A key neither primary nor fallback can't decrypt.
    let stranger = [0xcc; 32];
    let stranger_ct = encrypt_peer_secret("unknown", &stranger).expect("encrypt stranger");
    assert!(decrypt_peer_secret_rotating(&stranger_ct, &NEW_PRIMARY).is_err());
}
