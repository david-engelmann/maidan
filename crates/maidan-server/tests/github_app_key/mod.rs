//! A throwaway GitHub App key for the tests, generated when they run so no
//! private key is ever committed (`.gitignore` refuses `*.pem` for that
//! reason). Shared by `github_app_auth_e2e.rs` and the unit tests in
//! `src/github_app.rs`, which include this file by path.

// Each includer reads only some of the fields.
#![allow(dead_code)]

use std::sync::OnceLock;

use base64::Engine as _;

/// One RSA-2048 key in the two PEM forms the server accepts.
pub struct TestKey {
    /// `RSA PRIVATE KEY` (PKCS#1), the form GitHub downloads.
    pub pkcs1_pem: String,
    /// `PRIVATE KEY` (PKCS#8).
    pub pkcs8_pem: String,
    /// The PKCS#1 DER, for loading the key pair in a test.
    pub pkcs1_der: Vec<u8>,
}

/// The process's key, generated once.
pub fn test_key() -> &'static TestKey {
    static KEY: OnceLock<TestKey> = OnceLock::new();
    KEY.get_or_init(|| {
        use aws_lc_rs::encoding::AsDer as _;
        let pair = aws_lc_rs::rsa::KeyPair::generate(aws_lc_rs::rsa::KeySize::Rsa2048)
            .expect("generate an RSA test key");
        let pkcs8 = pair
            .as_der()
            .expect("export the test key")
            .as_ref()
            .to_vec();
        let pkcs1 = pkcs1_inside(&pkcs8);
        TestKey {
            pkcs1_pem: pem("RSA PRIVATE KEY", &pkcs1),
            pkcs8_pem: pem("PRIVATE KEY", &pkcs8),
            pkcs1_der: pkcs1,
        }
    })
}

/// The `RSAPrivateKey` a PKCS#8 `PrivateKeyInfo` wraps: the octet string after
/// the version and the algorithm identifier.
fn pkcs1_inside(pkcs8: &[u8]) -> Vec<u8> {
    let (tag, info, _) = tlv(pkcs8);
    assert_eq!(tag, 0x30, "PrivateKeyInfo is a SEQUENCE");
    let (_, _, rest) = tlv(info); // version
    let (_, _, rest) = tlv(rest); // algorithm
    let (tag, key, _) = tlv(rest);
    assert_eq!(tag, 0x04, "the private key is an OCTET STRING");
    key.to_vec()
}

/// One DER element: its tag, its contents and what follows it.
fn tlv(der: &[u8]) -> (u8, &[u8], &[u8]) {
    let tag = der[0];
    let (len, header) = if der[1] < 0x80 {
        (der[1] as usize, 2)
    } else {
        let n = (der[1] & 0x7f) as usize;
        let len = der[2..2 + n]
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | *b as usize);
        (len, 2 + n)
    };
    (tag, &der[header..header + len], &der[header + len..])
}

fn pem(label: &str, der: &[u8]) -> String {
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect();
    format!(
        "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
        lines.join("\n")
    )
}
