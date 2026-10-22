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

/// An in-memory stand-in for a cloud KMS custody ([`maidan_types::WorkspaceKeyCustody`]).
/// Each wrap makes a key of its own; a wrapped blob opens only under that key,
/// for the workspace key it was wrapped for, and not once the key is
/// destroyed, as a KMS binds a ciphertext to its key and encryption context.
pub mod fake_custody {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use maidan_types::{ContentKey, CustodyFuture, SealError, WorkspaceKeyCustody, WrappedKey};
    use uuid::Uuid;

    /// The `custody` the fake's rows name.
    pub const NAME: &str = "fake-kms";

    #[derive(Debug, Default)]
    struct State {
        /// Key id to (workspace, destroyed).
        keys: HashMap<String, (Uuid, bool)>,
        /// Wrapped blob to (key id, workspace key, the key's bytes).
        wrapped: HashMap<Vec<u8>, (String, Uuid, [u8; 32])>,
        unwraps: usize,
        refuse_destroy: bool,
    }

    #[derive(Debug, Default)]
    pub struct FakeCustody {
        state: Mutex<State>,
    }

    impl FakeCustody {
        pub fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        /// How many unwraps the fake has answered.
        pub fn unwraps(&self) -> usize {
            self.state.lock().map(|s| s.unwraps).unwrap_or(0)
        }

        /// The keys the fake made for `workspace_id`, and whether each is
        /// destroyed.
        pub fn keys_of(&self, workspace_id: Uuid) -> Vec<(String, bool)> {
            let Ok(state) = self.state.lock() else {
                return Vec::new();
            };
            let mut keys: Vec<(String, bool)> = state
                .keys
                .iter()
                .filter(|(_, (ws, _))| *ws == workspace_id)
                .map(|(id, (_, destroyed))| (id.clone(), *destroyed))
                .collect();
            keys.sort();
            keys
        }

        /// Refuse every destroy from now on, as a KMS that is down does.
        pub fn refuse_destroy(&self) {
            if let Ok(mut state) = self.state.lock() {
                state.refuse_destroy = true;
            }
        }

        fn key_id(kek_id: &str) -> Result<&str, SealError> {
            kek_id
                .strip_prefix(NAME)
                .and_then(|rest| rest.strip_prefix(':'))
                .ok_or_else(|| SealError::UnknownKek(kek_id.to_string()))
        }

        fn locked(&self) -> Result<std::sync::MutexGuard<'_, State>, SealError> {
            self.state
                .lock()
                .map_err(|_| SealError::Custody("fake custody poisoned".into()))
        }
    }

    impl WorkspaceKeyCustody for FakeCustody {
        fn name(&self) -> &'static str {
            NAME
        }

        fn wrap<'a>(
            &'a self,
            workspace_key_id: Uuid,
            workspace_id: Uuid,
            key: &'a ContentKey,
        ) -> CustodyFuture<'a, WrappedKey> {
            Box::pin(async move {
                let mut state = self.locked()?;
                let key_id = format!("key-{}", Uuid::now_v7());
                state.keys.insert(key_id.clone(), (workspace_id, false));
                let blob = ContentKey::generate().as_bytes().to_vec();
                state.wrapped.insert(
                    blob.clone(),
                    (key_id.clone(), workspace_key_id, *key.as_bytes()),
                );
                Ok(WrappedKey {
                    kek_id: format!("{NAME}:{key_id}"),
                    blob,
                })
            })
        }

        fn unwrap<'a>(
            &'a self,
            workspace_key_id: Uuid,
            wrapped: &'a WrappedKey,
        ) -> CustodyFuture<'a, ContentKey> {
            Box::pin(async move {
                let key_id = Self::key_id(&wrapped.kek_id)?;
                let mut state = self.locked()?;
                state.unwraps += 1;
                let destroyed = state.keys.get(key_id).is_none_or(|(_, d)| *d);
                match state.wrapped.get(&wrapped.blob) {
                    Some((owner, bound, bytes))
                        if !destroyed && owner == key_id && *bound == workspace_key_id =>
                    {
                        Ok(ContentKey::from_bytes(*bytes))
                    }
                    _ => Err(SealError::Custody("decrypt refused".into())),
                }
            })
        }

        fn destroy<'a>(&'a self, wrapped: &'a WrappedKey) -> CustodyFuture<'a, ()> {
            Box::pin(async move {
                let key_id = Self::key_id(&wrapped.kek_id)?;
                let mut state = self.locked()?;
                if state.refuse_destroy {
                    return Err(SealError::Custody("schedule deletion refused".into()));
                }
                if let Some((_, destroyed)) = state.keys.get_mut(key_id) {
                    *destroyed = true;
                }
                Ok(())
            })
        }
    }
}
