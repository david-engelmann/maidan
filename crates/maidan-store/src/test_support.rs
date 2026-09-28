//! Test-only helpers, behind the `test-support` feature.
//!
//! Production code passes the keyring it resolved from `MAIDAN_CONTENT_KEK`
//! (see [`crate::content_keyring`]); nothing outside tests reaches for the
//! public development KEK without saying so.

use std::sync::Arc;

use maidan_types::ContentKeyring;

/// The public development keyring. Anyone can unwrap what it wraps.
pub fn dev_keys() -> Arc<ContentKeyring> {
    Arc::new(ContentKeyring::insecure_dev())
}
