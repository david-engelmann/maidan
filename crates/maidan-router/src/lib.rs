//! Channel, thread, and message hierarchy resolution for Maidan.
//!
//! HTTP handlers and MCP fan-out call into this crate instead of duplicating
//! `get_thread` → `get_channel` chains against [`maidan_store::Store`].

pub mod error;
pub mod resolve;

pub use error::RouterError;
pub use resolve::{
    resolve_channel_context, resolve_message_chain, resolve_thread_context, ChannelContext,
    MessageChain, ThreadContext,
};
