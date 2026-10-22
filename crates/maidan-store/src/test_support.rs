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

/// Write a result the way a seat does, for tests and seeded fixtures: claim
/// the thread for `member` when it is unassigned (or use the lease `member`
/// already holds), write under that lease, and release a claim taken here,
/// so the thread is left as the caller found it.
#[async_trait::async_trait]
pub trait SetResultClaiming {
    async fn set_thread_result_claiming(
        &self,
        thread_id: maidan_types::ThreadId,
        member: maidan_types::MemberId,
        result: &serde_json::Value,
    ) -> Result<maidan_types::ThreadResult, crate::StoreError>;

    /// The lease `member` holds the thread under, claiming it first when it
    /// is unassigned: what a REST or MCP result write in a test sends as
    /// `claim_lease_id`. The claim stays.
    async fn claim_lease_for(
        &self,
        thread_id: maidan_types::ThreadId,
        member: maidan_types::MemberId,
    ) -> Result<maidan_types::ClaimLeaseId, crate::StoreError>;
}

#[async_trait::async_trait]
impl<S: crate::Store + ?Sized> SetResultClaiming for S {
    async fn set_thread_result_claiming(
        &self,
        thread_id: maidan_types::ThreadId,
        member: maidan_types::MemberId,
        result: &serde_json::Value,
    ) -> Result<maidan_types::ThreadResult, crate::StoreError> {
        let thread = self.get_thread(thread_id).await?;
        let (lease, claimed_here) = match (thread.assignee_id, thread.claim_lease_id) {
            (Some(holder), Some(lease)) if holder == member => (lease, false),
            _ => {
                let claim = self.claim_thread(thread_id, member).await?;
                match (claim.claimed, claim.thread.claim_lease_id) {
                    (true, Some(lease)) => (lease, true),
                    _ => {
                        return Err(crate::StoreError::ClaimLost(format!(
                            "thread {} is held by another member",
                            thread_id.0
                        )))
                    }
                }
            }
        };
        let written = self
            .set_thread_result(thread_id, member, lease, result)
            .await;
        if claimed_here {
            self.unassign_thread(thread_id).await?;
        }
        written
    }

    async fn claim_lease_for(
        &self,
        thread_id: maidan_types::ThreadId,
        member: maidan_types::MemberId,
    ) -> Result<maidan_types::ClaimLeaseId, crate::StoreError> {
        let thread = self.get_thread(thread_id).await?;
        if let (Some(holder), Some(lease)) = (thread.assignee_id, thread.claim_lease_id) {
            if holder == member {
                return Ok(lease);
            }
        }
        let claim = self.claim_thread(thread_id, member).await?;
        match (claim.claimed, claim.thread.claim_lease_id) {
            (true, Some(lease)) => Ok(lease),
            _ => Err(crate::StoreError::ClaimLost(format!(
                "thread {} is held by another member",
                thread_id.0
            ))),
        }
    }
}
