//! Core domain types for Maidan.
//!
//! Workspace, Member, Channel, Thread, Message, Mention, Vote, Reference,
//! Artifact, AuditEvent, plus typed IDs. Other crates depend on this one
//! for shared schema; nothing here depends on other Maidan crates.

pub mod erase;
pub mod events;
pub mod ids;
pub mod lsn;
pub mod models;
pub mod pack;
pub mod purge;
pub mod recipe;
pub mod usage;

pub use erase::*;
pub use events::*;
pub use ids::*;
pub use lsn::*;
pub use models::*;
pub use pack::*;
pub use purge::*;
pub use recipe::*;
pub use usage::*;
