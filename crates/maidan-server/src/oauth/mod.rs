//! OAuth 2.1 authorization server (`docs/OAuth.md`).
//!
//! Phase one serves the RFC 9728 protected-resource metadata for the MCP
//! endpoint and its 401 challenge, both off until `MAIDAN_PUBLIC_ORIGIN` is
//! set. Phase two adds the authorization-code flow with PKCE S256 only:
//! `GET /oauth/authorize` and `POST /oauth/token`, backed by the client
//! registry, codes and grants in migration 0150.
//!
//! It advertises no grant type, endpoint or registration method that is not
//! served: there is no dynamic client registration, by decision.
//!
//! Kept separate from `app_oauth.rs` (the installed-app code exchange) so
//! the two flows touch disjoint files.

pub mod authorize;
pub mod metadata;
pub mod token;

pub use metadata::{challenge, oauth_protected_resource};
