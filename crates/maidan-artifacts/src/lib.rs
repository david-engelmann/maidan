//! Content-addressed artifact store for Maidan.
//!
//! Artifacts are stored by sha256 of their bytes; identical content
//! produces a single physical file (dedup). The [`ArtifactStore`] trait
//! is backend-agnostic; [`LocalFsStore`] is the filesystem
//! implementation, suitable for local development and single-node
//! deployments. S3-compatible backends arrive in Cluster E.

pub mod error;
pub mod helpers;
pub mod localfs;
pub mod path;
pub mod s3;
pub mod s3_multipart;
pub mod sha;
pub mod store;

pub use error::ArtifactError;
pub use helpers::{put_attachment, put_code_dump, put_recording, put_screenshot, put_transcript};
pub use localfs::LocalFsStore;
pub use s3::{S3Config, S3Store};
pub use s3_multipart::{CompletedPart, MultipartUpload};
pub use sha::Sha256;
pub use store::{put_reader, ArtifactStore};
