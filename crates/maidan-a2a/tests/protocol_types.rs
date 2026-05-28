use maidan_a2a::{maidan_context_from_metadata, message_text, A2aMessage, TextPart};
use serde_json::json;

#[test]
fn maidan_context_parses_from_metadata() {
    let thread_id = uuid::Uuid::new_v4();
    let author_id = uuid::Uuid::new_v4();
    let meta = json!({
        "maidan": { "threadId": thread_id, "authorId": author_id }
    });
    let ctx = maidan_context_from_metadata(&Some(meta)).expect("context");
    assert_eq!(ctx.thread_id, thread_id);
    assert_eq!(ctx.author_id, author_id);
}

#[test]
fn message_text_joins_text_parts() {
    let msg = A2aMessage {
        role: "user".into(),
        parts: vec![
            TextPart {
                kind: "text".into(),
                text: "hello".into(),
            },
            TextPart {
                kind: "text".into(),
                text: "world".into(),
            },
        ],
        metadata: None,
    };
    assert_eq!(message_text(&msg).as_deref(), Some("hello\nworld"));
}
