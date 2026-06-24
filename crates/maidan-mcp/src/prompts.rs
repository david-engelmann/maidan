//! MCP prompts for thread lifecycle workflows.

use std::sync::Arc;

use maidan_store::Store;
use maidan_types::{ThreadId, ThreadState};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::McpError;

pub fn catalog() -> Vec<Value> {
    vec![json!({
        "name": "thread_workflow",
        "description": "Suggested agent steps for a thread based on its FSM state.",
        "arguments": [{
            "name": "thread_id",
            "description": "Thread UUID",
            "required": true
        }]
    })]
}

#[derive(Debug, Deserialize)]
struct ThreadWorkflowArgs {
    thread_id: uuid::Uuid,
}

pub async fn get(store: &Arc<dyn Store>, name: &str, args: &Value) -> Result<Value, McpError> {
    match name {
        "thread_workflow" => thread_workflow(store, args).await,
        other => Err(McpError::MethodNotFound(format!("prompt:{other}"))),
    }
}

async fn thread_workflow(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let parsed: ThreadWorkflowArgs = serde_json::from_value(args.clone())
        .map_err(|e| McpError::InvalidParams(format!("thread_workflow args: {e}")))?;
    let thread = store
        .get_thread(ThreadId(parsed.thread_id))
        .await
        .map_err(|_| McpError::NotFound)?;

    let body = match thread.state {
        ThreadState::Open => {
            "Thread is open. Gather context, then call start_review when ready for review."
        }
        ThreadState::InReview => "Thread is in review. Address feedback, then close when done.",
        ThreadState::Closed => "Thread is closed. Archive when no further work is needed.",
        ThreadState::Archived => "Thread is archived. No further lifecycle actions apply.",
    };

    Ok(json!({
        "description": format!("Workflow for thread {} ({})", thread.id.0, thread.state.as_str()),
        "messages": [{
            "role": "user",
            "content": {
                "type": "text",
                "text": body
            }
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_entries_are_well_formed() {
        let catalog = catalog();
        assert!(!catalog.is_empty(), "prompt catalog must not be empty");
        for prompt in &catalog {
            let name = prompt["name"].as_str().expect("prompt name is a string");
            assert!(!name.is_empty(), "prompt name must be non-empty");
            assert!(
                prompt["description"]
                    .as_str()
                    .is_some_and(|d| !d.is_empty()),
                "{name}: description must be a non-empty string"
            );
            // `arguments` (when present) must be an array of {name, required}.
            if let Some(args) = prompt.get("arguments") {
                let args = args.as_array().expect("arguments is an array");
                for arg in args {
                    assert!(arg["name"].as_str().is_some_and(|n| !n.is_empty()));
                    assert!(arg["required"].is_boolean());
                }
            }
        }
        // The documented `thread_workflow` prompt is present.
        assert!(catalog.iter().any(|p| p["name"] == "thread_workflow"));
    }
}
