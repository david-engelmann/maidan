//! OAuth 2.1 authorization server (`docs/OAuth.md`).
//!
//! Phase one serves the RFC 9728 protected-resource metadata for the MCP
//! endpoint and its 401 challenge, both off until `MAIDAN_PUBLIC_ORIGIN` is
//! set. Phase two adds the client registry, the consent page and the
//! authorization-code flow with PKCE S256 only: `GET /oauth/authorize` and
//! `POST /oauth/token`, backed by migrations 0150 to 0152.
//!
//! Phase three serves the RFC 8414 authorization-server document, naming
//! only the grant and endpoints that exist, and adds the RFC 9207 `iss`
//! parameter to the authorization response; refresh tokens and revocation
//! arrive in the rest of phase three. It advertises no grant type, endpoint
//! or registration method that is not served: there is no dynamic client
//! registration, by decision.
//!
//! Kept separate from `app_oauth.rs` (the installed-app code exchange) so
//! the two flows touch disjoint files.

pub mod authorize;
pub mod consent;
pub mod metadata;
pub mod registry;
pub mod token;

pub use metadata::{challenge, oauth_authorization_server, oauth_protected_resource};
