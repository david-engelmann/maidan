//! Web Push delivery (Cluster 366, Wave 1 #14, N1). VAPID (RFC 8292) + `aes128gcm`
//! payload encryption (RFC 8291 over RFC 8188), all RustCrypto — no openssl. The
//! notification router sends a Web Push message to a member's subscriptions when
//! the member has no live WebSocket connection.

use std::time::Duration;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use maidan_types::PushSubscription;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;

/// aes128gcm record size advertised in the content-coding header (RFC 8188). A
/// single small notification record fits well under this.
const RECORD_SIZE: u32 = 4096;
/// VAPID JWT validity window (RFC 8292 recommends ≤ 24h).
const VAPID_TTL_SECS: i64 = 12 * 60 * 60;
/// Web Push message TTL handed to the push service.
const PUSH_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, thiserror::Error)]
pub enum WebPushError {
    #[error("invalid key material: {0}")]
    InvalidKey(String),
    #[error("web push crypto error: {0}")]
    Crypto(String),
    #[error("push service returned status {0}")]
    Endpoint(u16),
    #[error("web push http error: {0}")]
    Http(String),
}

impl WebPushError {
    /// Whether the push service reports the subscription is gone (404/410) — the
    /// caller may prune it.
    pub fn is_gone(&self) -> bool {
        matches!(
            self,
            WebPushError::Endpoint(404) | WebPushError::Endpoint(410)
        )
    }
}

/// VAPID application-server identity (RFC 8292). Loaded from env; Web Push is
/// opt-in (like SMTP) — absent config means no sender is attached.
#[derive(Clone)]
pub struct WebPushConfig {
    signing_key: SigningKey,
    /// Uncompressed 65-byte VAPID public key (the `k=` parameter).
    public_key: Vec<u8>,
    /// `mailto:` or `https:` contact (the JWT `sub`).
    subject: String,
}

impl WebPushConfig {
    /// `VAPID_PRIVATE_KEY` (base64url 32-byte scalar), `VAPID_PUBLIC_KEY`
    /// (base64url 65-byte uncompressed point), `VAPID_SUBJECT`. Returns `None`
    /// unless all three are set and valid.
    pub fn from_env() -> Option<Self> {
        let priv_b64 = std::env::var("VAPID_PRIVATE_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let pub_b64 = std::env::var("VAPID_PUBLIC_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let subject = std::env::var("VAPID_SUBJECT")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        match Self::from_parts(&priv_b64, &pub_b64, subject) {
            Ok(config) => Some(config),
            Err(err) => {
                tracing::error!(%err, "VAPID config invalid; Web Push disabled");
                None
            }
        }
    }

    pub fn from_parts(
        priv_b64: &str,
        pub_b64: &str,
        subject: String,
    ) -> Result<Self, WebPushError> {
        let priv_bytes = URL_SAFE_NO_PAD
            .decode(priv_b64.trim())
            .map_err(|e| WebPushError::InvalidKey(e.to_string()))?;
        let signing_key = SigningKey::from_slice(&priv_bytes)
            .map_err(|e| WebPushError::InvalidKey(e.to_string()))?;
        let public_key = URL_SAFE_NO_PAD
            .decode(pub_b64.trim())
            .map_err(|e| WebPushError::InvalidKey(e.to_string()))?;
        if public_key.len() != 65 {
            return Err(WebPushError::InvalidKey(
                "VAPID public key must be a 65-byte uncompressed point".into(),
            ));
        }
        Ok(Self {
            signing_key,
            public_key,
            subject,
        })
    }
}

/// The `scheme://host[:port]` origin of a push endpoint (the VAPID JWT `aud`).
fn origin_of(url: &str) -> Result<String, WebPushError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| WebPushError::Http(e.to_string()))?;
    let scheme = parsed.scheme();
    let host = parsed
        .host_str()
        .ok_or_else(|| WebPushError::Http("push endpoint has no host".into()))?;
    match parsed.port() {
        Some(p) => Ok(format!("{scheme}://{host}:{p}")),
        None => Ok(format!("{scheme}://{host}")),
    }
}

/// Build the `Authorization: vapid t=<jwt>, k=<pubkey>` header for a push
/// endpoint's origin (RFC 8292). `now` is a parameter for testability.
fn vapid_authorization(
    config: &WebPushConfig,
    endpoint: &str,
    now: i64,
) -> Result<String, WebPushError> {
    let aud = origin_of(endpoint)?;
    let header_b64 = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = serde_json::json!({
        "aud": aud,
        "exp": now + VAPID_TTL_SECS,
        "sub": config.subject,
    });
    let payload_bytes =
        serde_json::to_vec(&claims).map_err(|e| WebPushError::Crypto(e.to_string()))?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload_bytes);
    let signing_input = format!("{header_b64}.{payload_b64}");
    let signature: Signature = config.signing_key.sign(signing_input.as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());
    let jwt = format!("{signing_input}.{sig_b64}");
    let k = URL_SAFE_NO_PAD.encode(&config.public_key);
    Ok(format!("vapid t={jwt}, k={k}"))
}

/// Encrypt `plaintext` for a subscription per RFC 8291 (aes128gcm content
/// encoding, RFC 8188). `as_secret` (the ephemeral application-server ECDH key)
/// and `salt` are parameters so the derivation is deterministically testable;
/// [`VapidWebPushSender::send`] passes fresh-random values.
fn encrypt_payload(
    ua_public_b64: &str,
    auth_secret_b64: &str,
    as_secret: &SecretKey,
    salt: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, WebPushError> {
    let ua_public_bytes = URL_SAFE_NO_PAD
        .decode(ua_public_b64.trim())
        .map_err(|e| WebPushError::InvalidKey(e.to_string()))?;
    let auth_secret = URL_SAFE_NO_PAD
        .decode(auth_secret_b64.trim())
        .map_err(|e| WebPushError::InvalidKey(e.to_string()))?;
    let ua_public = PublicKey::from_sec1_bytes(&ua_public_bytes)
        .map_err(|e| WebPushError::InvalidKey(e.to_string()))?;

    let as_public = as_secret.public_key();
    let as_public_bytes = as_public.to_encoded_point(false).as_bytes().to_vec();

    // ECDH shared secret: server_private × ua_public.
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_public.as_affine());
    let ecdh_secret = shared.raw_secret_bytes();

    // RFC 8291 §3.4: IKM = HKDF(salt = auth_secret, ikm = ecdh_secret,
    //   info = "WebPush: info" || 0x00 || ua_public || as_public, L = 32).
    let mut key_info = Vec::with_capacity(14 + 65 + 65);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(&ua_public_bytes);
    key_info.extend_from_slice(&as_public_bytes);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&auth_secret), ecdh_secret.as_slice())
        .expand(&key_info, &mut ikm)
        .map_err(|e| WebPushError::Crypto(e.to_string()))?;

    // RFC 8188: derive the content-encryption key + nonce from the record salt.
    let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|e| WebPushError::Crypto(e.to_string()))?;
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|e| WebPushError::Crypto(e.to_string()))?;

    // Single, final record: plaintext || 0x02 delimiter (RFC 8188 §2.1).
    let mut record = plaintext.to_vec();
    record.push(0x02);
    let cipher =
        Aes128Gcm::new_from_slice(&cek).map_err(|e| WebPushError::Crypto(e.to_string()))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), record.as_ref())
        .map_err(|e| WebPushError::Crypto(e.to_string()))?;

    // aes128gcm header: salt(16) || rs(4 BE) || idlen(1) || keyid(as_public) || ct.
    let mut body = Vec::with_capacity(16 + 4 + 1 + as_public_bytes.len() + ciphertext.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(as_public_bytes.len() as u8);
    body.extend_from_slice(&as_public_bytes);
    body.extend_from_slice(&ciphertext);
    Ok(body)
}

/// Sends an encrypted Web Push message to one subscription (Cluster 366).
#[async_trait::async_trait]
pub trait WebPushSender: Send + Sync {
    async fn send(&self, sub: &PushSubscription, payload: &[u8]) -> Result<(), WebPushError>;
}

/// The production sender: VAPID-authorized, aes128gcm-encrypted POST to the push
/// service (RFC 8030 + 8291 + 8292).
pub struct VapidWebPushSender {
    config: WebPushConfig,
    client: reqwest::Client,
}

impl VapidWebPushSender {
    pub fn new(config: WebPushConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client }
    }
}

#[async_trait::async_trait]
impl WebPushSender for VapidWebPushSender {
    async fn send(&self, sub: &PushSubscription, payload: &[u8]) -> Result<(), WebPushError> {
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        let as_secret = SecretKey::random(&mut OsRng);
        let body = encrypt_payload(&sub.p256dh, &sub.auth, &as_secret, &salt, payload)?;
        let auth =
            vapid_authorization(&self.config, &sub.endpoint, chrono::Utc::now().timestamp())?;
        let resp = self
            .client
            .post(&sub.endpoint)
            .header("Authorization", auth)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("TTL", PUSH_TTL_SECS.to_string())
            .body(body)
            .send()
            .await
            .map_err(|e| WebPushError::Http(e.to_string()))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(WebPushError::Endpoint(resp.status().as_u16()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{signature::Verifier, VerifyingKey};

    /// A fresh VAPID keypair (base64url) for tests.
    fn gen_vapid() -> (String, String) {
        let sk = SecretKey::random(&mut OsRng);
        let priv_b64 = URL_SAFE_NO_PAD.encode(sk.to_bytes());
        let pub_b64 = URL_SAFE_NO_PAD.encode(sk.public_key().to_encoded_point(false).as_bytes());
        (priv_b64, pub_b64)
    }

    #[test]
    fn vapid_authorization_is_a_verifiable_es256_jwt() {
        let (priv_b64, pub_b64) = gen_vapid();
        let config =
            WebPushConfig::from_parts(&priv_b64, &pub_b64, "mailto:ops@example.com".into())
                .expect("config");
        let header =
            vapid_authorization(&config, "https://push.example.com/abc?x=1", 1_000_000).unwrap();

        // Parse `vapid t=<jwt>, k=<pub>`.
        let rest = header.strip_prefix("vapid t=").expect("prefix");
        let (jwt, k_part) = rest.split_once(", k=").expect("k param");
        let pub_bytes = URL_SAFE_NO_PAD.decode(k_part).unwrap();
        assert_eq!(pub_bytes, URL_SAFE_NO_PAD.decode(&pub_b64).unwrap());

        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let sig_bytes = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        let sig = Signature::from_slice(&sig_bytes).unwrap();
        let vk = VerifyingKey::from_sec1_bytes(&pub_bytes).unwrap();
        vk.verify(signing_input.as_bytes(), &sig)
            .expect("signature verifies");

        // Claims: aud = origin, sub carried through, exp in the future.
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://push.example.com");
        assert_eq!(claims["sub"], "mailto:ops@example.com");
        assert_eq!(claims["exp"], 1_000_000 + VAPID_TTL_SECS);
    }

    /// Decrypt an aes128gcm Web Push body as the user agent would (RFC 8291),
    /// independently following the spec so the round trip validates the sender's
    /// key-derivation *ordering*, not just its self-consistency.
    fn ua_decrypt(ua_secret: &SecretKey, auth_secret: &[u8], body: &[u8]) -> Vec<u8> {
        let salt = &body[0..16];
        let idlen = body[20] as usize;
        let as_public_bytes = &body[21..21 + idlen];
        let ciphertext = &body[21 + idlen..];

        let ua_public_bytes = ua_secret
            .public_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        let as_public = PublicKey::from_sec1_bytes(as_public_bytes).unwrap();
        let shared =
            p256::ecdh::diffie_hellman(ua_secret.to_nonzero_scalar(), as_public.as_affine());

        let mut key_info = Vec::new();
        key_info.extend_from_slice(b"WebPush: info\0");
        key_info.extend_from_slice(&ua_public_bytes);
        key_info.extend_from_slice(as_public_bytes);
        let mut ikm = [0u8; 32];
        Hkdf::<Sha256>::new(Some(auth_secret), shared.raw_secret_bytes().as_slice())
            .expand(&key_info, &mut ikm)
            .unwrap();

        let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);
        let mut cek = [0u8; 16];
        hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
            .unwrap();
        let mut nonce = [0u8; 12];
        hk.expand(b"Content-Encoding: nonce\0", &mut nonce).unwrap();

        let cipher = Aes128Gcm::new_from_slice(&cek).unwrap();
        let mut plain = cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext)
            .expect("decrypts");
        // Strip the RFC 8188 record delimiter.
        assert_eq!(plain.pop(), Some(0x02));
        plain
    }

    /// The authoritative interop check: reproduce the exact ciphertext from RFC
    /// 8291 Appendix A ("Push Message Encryption Example"). A round-trip alone can't
    /// catch a wrong-but-consistent info label or delimiter; this pins the bytes.
    #[test]
    fn encrypt_payload_matches_rfc8291_appendix_a() {
        let ua_public_b64 =
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
        let auth_b64 = "BTBZMqHH6r4Tts7J_aSIgg";
        let as_private_b64 = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
        let salt_b64 = "DGv6ra1nlYgDCS1FRnbzlw";
        let plaintext = b"When I grow up, I want to be a watermelon";
        let expected = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

        let as_priv_bytes = URL_SAFE_NO_PAD.decode(as_private_b64).unwrap();
        let as_secret = SecretKey::from_slice(&as_priv_bytes).unwrap();
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&URL_SAFE_NO_PAD.decode(salt_b64).unwrap());

        let body = encrypt_payload(ua_public_b64, auth_b64, &as_secret, &salt, plaintext).unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.encode(&body),
            expected,
            "must reproduce the RFC 8291 Appendix A ciphertext byte-for-byte"
        );
    }

    #[test]
    fn encrypt_payload_round_trips_through_a_ua_decrypt() {
        // The "user agent" (subscription) keypair + auth secret.
        let ua_secret = SecretKey::random(&mut OsRng);
        let ua_public_b64 =
            URL_SAFE_NO_PAD.encode(ua_secret.public_key().to_encoded_point(false).as_bytes());
        let mut auth_secret = [0u8; 16];
        OsRng.fill_bytes(&mut auth_secret);
        let auth_b64 = URL_SAFE_NO_PAD.encode(auth_secret);

        // The server-side ephemeral key + salt.
        let as_secret = SecretKey::random(&mut OsRng);
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);

        let plaintext = b"When I grow up, I want to be a watermelon";
        let body =
            encrypt_payload(&ua_public_b64, &auth_b64, &as_secret, &salt, plaintext).unwrap();

        // Header framing (RFC 8188): salt, rs=4096, idlen=65, keyid=as_public.
        assert_eq!(&body[0..16], &salt);
        assert_eq!(&body[16..20], &RECORD_SIZE.to_be_bytes());
        assert_eq!(body[20], 65);
        assert_eq!(
            &body[21..86],
            as_secret.public_key().to_encoded_point(false).as_bytes()
        );

        let recovered = ua_decrypt(&ua_secret, &auth_secret, &body);
        assert_eq!(recovered, plaintext);
    }
}
