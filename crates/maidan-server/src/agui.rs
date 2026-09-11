//! An AG-UI door on Maidan's event stream (Cluster 369, Wave 2 #17, H1). AG-UI
//! (the Agent-User Interaction protocol) is the human-UI/IDE-facing event
//! protocol: a run emits `RUN_STARTED` → `TEXT_MESSAGE_*` / `TOOL_CALL_*` /
//! `STEP_*` → `RUN_FINISHED | RUN_ERROR`. This module maps Maidan's domain events
//! (a **thread is a run**) to AG-UI events so an AG-UI-compatible frontend can
//! render agent activity off the existing WS/SSE bus — no CopilotKit, no new
//! runtime. F3 (agent↔tool) stays MCP; this is the human-facing door.
//!
//! Output-direction only for now: Maidan `Event` → AG-UI events. The input
//! direction (a UI sending `RunAgentInput`, interrupts, `editedArgs` = full
//! replace) is a follow-up — the room already has REST for those mutations.

use maidan_types::{ContentBlock, Event, Message};
use serde::Serialize;

/// An AG-UI protocol event. Serializes to the AG-UI JSON shape — a
/// `SCREAMING_SNAKE_CASE` `type` tag plus `camelCase` fields.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgUiEvent {
    #[serde(rename_all = "camelCase")]
    RunStarted { thread_id: String, run_id: String },
    #[serde(rename_all = "camelCase")]
    RunFinished { thread_id: String, run_id: String },
    #[serde(rename_all = "camelCase")]
    RunError {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    StepStarted { step_name: String },
    #[serde(rename_all = "camelCase")]
    StepFinished { step_name: String },
    #[serde(rename_all = "camelCase")]
    TextMessageStart { message_id: String, role: String },
    #[serde(rename_all = "camelCase")]
    TextMessageContent { message_id: String, delta: String },
    #[serde(rename_all = "camelCase")]
    TextMessageEnd { message_id: String },
    #[serde(rename_all = "camelCase")]
    ToolCallStart {
        tool_call_id: String,
        tool_call_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallArgs { tool_call_id: String, delta: String },
    #[serde(rename_all = "camelCase")]
    ToolCallEnd { tool_call_id: String },
    #[serde(rename_all = "camelCase")]
    ToolCallResult {
        message_id: String,
        tool_call_id: String,
        content: String,
    },
    /// Anything without a first-class AG-UI mapping rides a `CUSTOM` event, so an
    /// AG-UI UI can still surface it (Maidan-specific: `mention`, `landed`, …).
    #[serde(rename_all = "camelCase")]
    Custom {
        name: String,
        value: serde_json::Value,
    },
}

/// A Maidan message → its AG-UI text + tool-call events. The text message is one
/// START / CONTENT / END triple (Maidan messages are complete, not streamed);
/// each Cluster-173 `ToolUse` block becomes a `TOOL_CALL_*` triple and each
/// `ToolResult` a `TOOL_CALL_RESULT`.
fn message_events(message: &Message) -> Vec<AgUiEvent> {
    let mid = message.id.0.to_string();
    let mut events = vec![AgUiEvent::TextMessageStart {
        message_id: mid.clone(),
        role: "assistant".to_string(),
    }];
    if !message.body.is_empty() {
        events.push(AgUiEvent::TextMessageContent {
            message_id: mid.clone(),
            delta: message.body.clone(),
        });
    }
    events.push(AgUiEvent::TextMessageEnd {
        message_id: mid.clone(),
    });
    if let Some(blocks) = &message.content {
        for block in blocks {
            match block {
                ContentBlock::ToolUse { id, name, input } => {
                    events.push(AgUiEvent::ToolCallStart {
                        tool_call_id: id.clone(),
                        tool_call_name: name.clone(),
                        parent_message_id: Some(mid.clone()),
                    });
                    events.push(AgUiEvent::ToolCallArgs {
                        tool_call_id: id.clone(),
                        delta: input.to_string(),
                    });
                    events.push(AgUiEvent::ToolCallEnd {
                        tool_call_id: id.clone(),
                    });
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    events.push(AgUiEvent::ToolCallResult {
                        message_id: mid.clone(),
                        tool_call_id: tool_use_id.clone(),
                        content: content.clone(),
                    });
                }
                _ => {}
            }
        }
    }
    events
}

/// Map one Maidan domain [`Event`] to zero or more AG-UI events (Cluster 369). A
/// thread **is** a run: `ThreadCreated` → `RUN_STARTED`, a terminal
/// `ThreadStateChanged` → `RUN_FINISHED`, a non-terminal one → `STEP_STARTED`,
/// `ClaimFailed` → `RUN_ERROR`, `MessagePosted` → the text + tool-call events. A
/// few Maidan-specific facts ride `CUSTOM`; the rest map to nothing.
pub fn agui_events_for(event: &Event) -> Vec<AgUiEvent> {
    match event {
        Event::ThreadCreated { thread, .. } => {
            let tid = thread.id.0.to_string();
            vec![AgUiEvent::RunStarted {
                thread_id: tid.clone(),
                run_id: tid,
            }]
        }
        Event::ThreadStateChanged {
            thread_id,
            to_state,
            ..
        } => {
            let tid = thread_id.0.to_string();
            if to_state.is_terminal() {
                vec![AgUiEvent::RunFinished {
                    thread_id: tid.clone(),
                    run_id: tid,
                }]
            } else {
                vec![AgUiEvent::StepStarted {
                    step_name: format!("state:{}", to_state.as_str()),
                }]
            }
        }
        Event::ClaimFailed { reason, .. } => vec![AgUiEvent::RunError {
            message: format!("run stopped: budget ({reason}) exceeded"),
            code: Some("budget_exceeded".to_string()),
        }],
        Event::MessagePosted { message, .. } => message_events(message),
        Event::ThreadLanded {
            thread_id,
            repo,
            pr_number,
            ..
        } => vec![AgUiEvent::Custom {
            name: "landed".to_string(),
            value: serde_json::json!({
                "threadId": thread_id.0.to_string(),
                "repo": repo,
                "prNumber": pr_number,
            }),
        }],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use maidan_types::*;
    use uuid::Uuid;

    fn thread(state: ThreadState) -> Thread {
        Thread {
            id: ThreadId(Uuid::new_v4()),
            channel_id: ChannelId(Uuid::new_v4()),
            parent_thread_id: None,
            title: Some("t".into()),
            state,
            assignee_id: None,
            assignment_expires_at: None,
            claim_lease_id: None,
            work_started_at: None,
            owner_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        }
    }

    fn type_of(e: &AgUiEvent) -> String {
        serde_json::to_value(e).unwrap()["type"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn thread_created_is_a_run_started_with_run_id_equal_to_thread_id() {
        let t = thread(ThreadState::Open);
        let ev = Event::ThreadCreated {
            occurred_at: Utc::now(),
            workspace_id: WorkspaceId(Uuid::new_v4()),
            channel_id: t.channel_id,
            thread: t.clone(),
        };
        let out = agui_events_for(&ev);
        assert_eq!(out.len(), 1);
        let v = serde_json::to_value(&out[0]).unwrap();
        assert_eq!(v["type"], "RUN_STARTED");
        assert_eq!(v["threadId"], t.id.0.to_string());
        assert_eq!(v["runId"], t.id.0.to_string());
    }

    #[test]
    fn terminal_transition_finishes_the_run_and_non_terminal_is_a_step() {
        let t = thread(ThreadState::Closed);
        let closed = Event::ThreadStateChanged {
            occurred_at: Utc::now(),
            workspace_id: WorkspaceId(Uuid::new_v4()),
            channel_id: t.channel_id,
            thread_id: t.id,
            actor_id: MemberId(Uuid::new_v4()),
            from_state: ThreadState::Open,
            to_state: ThreadState::Closed,
            thread: t.clone(),
        };
        assert_eq!(type_of(&agui_events_for(&closed)[0]), "RUN_FINISHED");

        let reviewing = Event::ThreadStateChanged {
            occurred_at: Utc::now(),
            workspace_id: WorkspaceId(Uuid::new_v4()),
            channel_id: t.channel_id,
            thread_id: t.id,
            actor_id: MemberId(Uuid::new_v4()),
            from_state: ThreadState::Open,
            to_state: ThreadState::InReview,
            thread: t,
        };
        let step = agui_events_for(&reviewing);
        assert_eq!(type_of(&step[0]), "STEP_STARTED");
        let v = serde_json::to_value(&step[0]).unwrap();
        assert_eq!(v["stepName"], "state:in_review");
    }

    #[test]
    fn claim_failed_is_a_run_error() {
        let t = thread(ThreadState::Open);
        let ev = Event::ClaimFailed {
            occurred_at: Utc::now(),
            workspace_id: WorkspaceId(Uuid::new_v4()),
            channel_id: t.channel_id,
            thread_id: t.id,
            member_id: MemberId(Uuid::new_v4()),
            reason: "tokens".into(),
            thread: t,
        };
        let v = serde_json::to_value(&agui_events_for(&ev)[0]).unwrap();
        assert_eq!(v["type"], "RUN_ERROR");
        assert_eq!(v["code"], "budget_exceeded");
        assert!(v["message"].as_str().unwrap().contains("tokens"));
    }

    #[test]
    fn message_maps_to_text_events_plus_tool_calls() {
        let mid = MessageId(Uuid::new_v4());
        let message = Message {
            id: mid,
            thread_id: ThreadId(Uuid::new_v4()),
            author_id: MemberId(Uuid::new_v4()),
            body: "running the tool".into(),
            metadata: serde_json::json!({}),
            content: Some(vec![
                ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "search".into(),
                    input: serde_json::json!({ "q": "x" }),
                },
                ContentBlock::ToolResult {
                    tool_use_id: "call_1".into(),
                    content: "3 hits".into(),
                    is_error: false,
                },
            ]),
            posted_at: Utc::now(),
            edited_at: None,
            tombstoned_at: None,
        };
        let ev = Event::MessagePosted {
            occurred_at: Utc::now(),
            workspace_id: WorkspaceId(Uuid::new_v4()),
            channel_id: ChannelId(Uuid::new_v4()),
            thread_id: message.thread_id,
            dm_conversation_id: None,
            message,
        };
        let out = agui_events_for(&ev);
        let types: Vec<String> = out.iter().map(type_of).collect();
        assert_eq!(
            types,
            vec![
                "TEXT_MESSAGE_START",
                "TEXT_MESSAGE_CONTENT",
                "TEXT_MESSAGE_END",
                "TOOL_CALL_START",
                "TOOL_CALL_ARGS",
                "TOOL_CALL_END",
                "TOOL_CALL_RESULT",
            ]
        );
        // The text events carry the message id; the tool events the call id.
        let start = serde_json::to_value(&out[0]).unwrap();
        assert_eq!(start["messageId"], mid.0.to_string());
        assert_eq!(start["role"], "assistant");
        let tool_start = serde_json::to_value(&out[3]).unwrap();
        assert_eq!(tool_start["toolCallId"], "call_1");
        assert_eq!(tool_start["toolCallName"], "search");
        assert_eq!(tool_start["parentMessageId"], mid.0.to_string());
    }

    #[test]
    fn an_empty_body_omits_the_content_event() {
        let message = Message {
            id: MessageId(Uuid::new_v4()),
            thread_id: ThreadId(Uuid::new_v4()),
            author_id: MemberId(Uuid::new_v4()),
            body: String::new(),
            metadata: serde_json::json!({}),
            content: None,
            posted_at: Utc::now(),
            edited_at: None,
            tombstoned_at: None,
        };
        let out = message_events(&message);
        let types: Vec<String> = out.iter().map(type_of).collect();
        assert_eq!(types, vec!["TEXT_MESSAGE_START", "TEXT_MESSAGE_END"]);
    }
}
