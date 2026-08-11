//! AuthN/AuthZ for Maidan: API token hashing, capability vocabulary, and
//! bearer resolution against the store.

pub mod access;
pub mod capability;
pub mod context;
pub mod error;
pub mod peer_secret;
pub mod resolve;
pub mod token;

pub use access::{
    can_access_channel, can_access_thread, ensure_channel_access, ensure_dm_participant,
    ensure_message_access, ensure_thread_access,
};
pub use capability::*;
pub use context::AuthContext;
pub use error::AuthError;
pub use peer_secret::{
    decrypt_fallback_keys_from_env, decrypt_peer_secret, decrypt_peer_secret_multi,
    decrypt_peer_secret_rotating, encrypt_peer_secret, encryption_key_from_env,
    init_decrypt_fallback_keys, PeerSecretError,
};
pub use resolve::{resolve_bearer, resolve_peer_bearer};
pub use token::{hash_secret, TokenSecret};
