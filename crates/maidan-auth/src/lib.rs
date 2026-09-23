//! AuthN/AuthZ for Maidan: API token hashing, capability vocabulary, and
//! bearer resolution against the store.

pub mod access;
pub mod capability;
pub mod capability_set;
pub mod context;
pub mod egress;
pub mod error;
pub mod export_sign;
pub mod peer_secret;
pub mod resolve;
pub mod token;

pub use access::{
    authorize_message, authorize_thread, can_access_channel, can_access_thread,
    ensure_channel_access, ensure_dm_participant, ensure_message_access, ensure_thread_access,
    private_channel_deny_set, resolve_wasi_handler_target, MessageScope, ThreadScope,
    WasiTargetError,
};
pub use capability::*;
pub use capability_set::{
    attenuate, attenuate_expiry, expand_set, held_sets, is_named_set, named_sets,
    progressive_grant, CapabilitySet, AGENT_WORKER, HUMAN_ADMIN,
};
pub use context::AuthContext;
pub use egress::*;
pub use error::AuthError;
pub use export_sign::{
    export_signing_key_from_env, export_verify_keys_from_env, sign_export, verify_export,
    ExportSignError, ExportSigningKey, ExportVerifyError,
};
pub use peer_secret::{
    decrypt_fallback_keys_from_env, decrypt_peer_secret, decrypt_peer_secret_multi,
    decrypt_peer_secret_rotating, encrypt_peer_secret, encryption_key_from_env,
    init_decrypt_fallback_keys, PeerSecretError,
};
pub use resolve::{resolve_bearer, resolve_peer_bearer};
pub use token::{hash_secret, ShareTicketSecret, TokenSecret};
