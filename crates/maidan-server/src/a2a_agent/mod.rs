//! The A2A protocol v1.0 agent: the JSON-RPC (§9) and HTTP+JSON (§11)
//! bindings over one set of transport-neutral operations ([`ops`]), plus the
//! Agent Card (§8).
//!
//! A Maidan A2A task is one message delivery. `SendMessage` posts the
//! caller's message into the thread its `contextId` names and returns a
//! completed task; pending approval gates surface as `input-required` tasks.
//! Task rows carry no message words: history is rendered from the sealed
//! message log on every read, so shredding a message removes it from A2A too.

mod card;
mod error;
mod jsonrpc;
pub(crate) mod ops;
mod push;
mod rest;
mod version;

pub use card::{agent_card, A2aCardConfig, AgentCard};
pub use jsonrpc::json_rpc;
pub use rest::{
    rest_create_push_config, rest_delete_push_config, rest_extended_agent_card,
    rest_get_push_config, rest_list_push_configs, rest_list_tasks, rest_message, rest_task_get,
    rest_task_post,
};
pub(crate) use version::check_grpc_version;
