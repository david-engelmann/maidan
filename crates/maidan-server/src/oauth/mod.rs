//! OAuth 2.1 authorization server, phase one: discovery and metadata.
//!
//! This module serves the RFC 8414 authorization-server metadata and the
//! RFC 9728 protected-resource metadata. Phase one advertises no grant
//! type, endpoint, or registration method that is not served: the token,
//! authorize, and revocation endpoints land in later phases (see
//! `docs/OAuth.md`).
//!
//! Kept separate from `app_oauth.rs` (the installed-app code exchange) so
//! the two flows touch disjoint files.

pub mod metadata;

pub use metadata::{oauth_authorization_server, oauth_protected_resource};
