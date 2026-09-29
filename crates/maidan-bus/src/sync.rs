//! The lock the fan-out uses: std's, or loom's in the crate's own tests under
//! the `loom` feature, so the model tests in `sharded.rs` can explore every
//! interleaving of it. A build that is not this crate's test build always
//! gets std's, so enabling the feature elsewhere cannot put loom's types
//! outside `loom::model`.

#[cfg(all(test, feature = "loom"))]
pub(crate) use loom::sync::{Mutex, MutexGuard};
#[cfg(not(all(test, feature = "loom")))]
pub(crate) use std::sync::{Mutex, MutexGuard};

/// A step on a tokio broadcast channel (send, subscribe). Channel operations
/// are atomic but are not loom operations, so in the loom tests each one is
/// preceded by a yield: the scheduler may switch threads there, which exposes
/// the window between releasing the shard lock and using a channel taken
/// under it. Compiles to nothing otherwise.
#[inline(always)]
pub(crate) fn channel_step() {
    #[cfg(all(test, feature = "loom"))]
    loom::thread::yield_now();
}
