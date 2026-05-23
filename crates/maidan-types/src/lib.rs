//! Core domain types for Maidan.
//!
//! Workspace, Member, Channel, Thread, Message, Mention, Vote, Reference,
//! Artifact, AuditEvent, plus typed IDs. Other crates depend on this one
//! for shared schema; nothing here depends on other Maidan crates.

pub mod events;
pub mod ids;
pub mod models;

pub use events::*;
pub use ids::*;
pub use models::*;
