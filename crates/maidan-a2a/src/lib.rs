//! Maidan federation wire format plus Google A2A protocol v1.0 client types.

pub mod batch;
pub mod client;
pub mod envelope;
pub mod error;
pub mod outbound;
pub mod peer;
pub mod protocol;
pub mod tasks;

#[cfg(test)]
pub mod test_support;

pub use batch::{FederatedEventBatch, MAX_FEDERATION_BATCH_SIZE};
pub use client::A2aClient;
pub use envelope::FederationEnvelope;
pub use error::{A2aClientError, FederationError};
pub use outbound::Outbound;
pub use peer::{validate_base_url, validate_peer_name, NewPeer, Peer};
pub use protocol::{
    maidan_context_from_metadata, message_text, A2aMessage, GetTaskRequest, JsonRpcId, TextPart,
    JsonRpcRequest, JsonRpcResponse, MaidanA2aContext, SendMessageRequest, SendMessageResponse,
    Task, TaskStatus, METHOD_GET_TASK, METHOD_SEND_MESSAGE, TASK_STATE_COMPLETED,
};
pub use tasks::TaskRegistry;
