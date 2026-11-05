//! OAuth 2.1 authorization server (`docs/OAuth.md`).
//!
//! Phase one serves the RFC 9728 protected-resource metadata for the MCP
//! endpoint and its 401 challenge, both off until `MAIDAN_PUBLIC_ORIGIN` is
//! set. Phase two adds the client registry, the consent page and the
//! authorization-code flow with PKCE S256 only: `GET /oauth/authorize` and
//! `POST /oauth/token`, backed by migrations 0150 to 0152.
//!
//! Phase three serves the RFC 8414 authorization-server document and adds
//! the RFC 9207 `iss` parameter to the authorization response, refresh
//! tokens with rotation and lineage revocation on reuse, `POST /oauth/revoke`
//! (RFC 7009), HTTP Basic client authentication, and RFC 6749 error bodies
//! on the token and revocation endpoints. Every token is bound to the MCP
//! resource (RFC 8707): the bearer middleware accepts it on `/mcp/streamable`
//! only. With no `MAIDAN_PUBLIC_ORIGIN` the whole server is off. It
//! advertises no grant type, endpoint or registration method that is not
//! served.
//!
//! RFC 7591 dynamic client registration (`register`, decided 2026-11-05) is
//! off unless `MAIDAN_OAUTH_DYNAMIC_REGISTRATION` is set: public clients only,
//! with redirects that land on the person's own machine.
//!
//! Kept separate from `app_oauth.rs` (the installed-app code exchange) so
//! the two flows touch disjoint files.

pub mod authorize;
pub mod client_auth;
pub mod consent;
pub mod error;
pub mod grants;
pub mod metadata;
mod page;
pub mod register;
pub mod registry;
pub mod revoke;
pub mod token;

pub use metadata::{challenge, oauth_authorization_server, oauth_protected_resource};
