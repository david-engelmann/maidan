//! Content-addressed artifact store for Maidan.
//!
//! Artifacts are stored by sha256 of their bytes; identical content
//! produces a single physical file (dedup). The [`ArtifactStore`] trait
//! is backend-agnostic; [`LocalFsStore`] is the filesystem
//! implementation, suitable for local development and single-node
//! deployments. S3-compatible backends arrive in Cluster E.

pub mod error;
pub mod localfs;
pub mod sha;
pub mod store;

pub use error::ArtifactError;
pub use localfs::LocalFsStore;
pub use sha::Sha256;
pub use store::ArtifactStore;
