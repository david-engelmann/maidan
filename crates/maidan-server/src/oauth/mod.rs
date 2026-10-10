//! OAuth 2.1 authorization server, phase one: discovery and metadata.
//!
//! Phase one serves the RFC 9728 protected-resource metadata for the MCP
//! endpoint and its 401 challenge, both off until `MAIDAN_PUBLIC_ORIGIN` is
//! set. It advertises no grant type, endpoint or registration method: the
//! authorization-server document arrives with the token endpoint (see
//! `docs/OAuth.md`).
//!
//! Kept separate from `app_oauth.rs` (the installed-app code exchange) so
//! the two flows touch disjoint files.

pub mod metadata;

pub use metadata::{challenge, oauth_protected_resource};
