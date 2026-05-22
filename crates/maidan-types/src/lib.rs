//! Core domain types for Maidan.
//!
//! Member, Channel, Thread, Message, and typed IDs. Other crates depend on
//! this one for shared schema; nothing in this crate depends on other Maidan
//! crates.

pub mod ids;

pub use ids::*;
