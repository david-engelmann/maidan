use maidan_a2a::{message_text, Message, Part, Role, SendMessageRequest};
use serde_json::json;

#[test]
fn a_spec_send_message_request_parses() {
    // The request body from the A2A v1.0 spec's REST example (§11.4).
    let req: SendMessageRequest = serde_json::from_value(json!({
        "message": {
            "messageId": "uuid",
            "role": "ROLE_USER",
            "parts": [{"text": "Hello"}]
        },
        "configuration": {"acceptedOutputModes": ["text/plain"]}
    }))
    .expect("spec example parses");
    assert_eq!(req.message.role, Role::User);
    assert_eq!(
        req.configuration
            .expect("configuration")
            .accepted_output_modes,
        vec!["text/plain".to_string()]
    );
}

#[test]
fn message_text_joins_text_parts() {
    let msg = Message {
        message_id: "m1".into(),
        context_id: None,
        task_id: None,
        role: Role::User,
        parts: vec![Part::text("hello"), Part::text("world")],
        metadata: None,
        extensions: vec![],
        reference_task_ids: vec![],
    };
    assert_eq!(message_text(&msg).as_deref(), Some("hello\nworld"));
}
