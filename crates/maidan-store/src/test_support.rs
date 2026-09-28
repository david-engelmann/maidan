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

/// What a testcontainers test does when its container will not start.
///
/// A test may skip only when there is no Docker daemon at all. With one
/// running, a start failure is real (an image that can no longer be pulled, a
/// readiness message that never came) and has to fail the test. Treating
/// every start error as "no Docker" let the MinIO tests pass without
/// running.
#[cfg(feature = "test-support")]
pub mod docker {
    use std::fmt::Display;

    /// Whether the Docker daemon testcontainers would use answers a ping.
    /// That client honours `DOCKER_HOST` and the rootless and Docker Desktop
    /// socket fallbacks.
    pub async fn daemon_answers() -> bool {
        match testcontainers::core::client::docker_client_instance().await {
            Ok(docker) => docker.ping().await.is_ok(),
            Err(_) => false,
        }
    }

    /// Call with the error from a container that failed to start. Returns,
    /// so the caller can skip, only when no Docker daemon answers; panics with
    /// `err` otherwise.
    pub async fn skip_start_failure(err: impl Display) {
        assert!(
            !daemon_answers().await,
            "the container failed to start with a Docker daemon running: {err}"
        );
        eprintln!("skipping: no Docker daemon ({err})");
    }
}
