//! The first frame on `/ws/subscribe` arrives before the caller's credential is
//! checked. Whatever the frame says, decoding it does not panic; a token the
//! fuzzer wrote never verifies; a frame's own cursor is never negative and
//! never replays without a workspace; and the filter the server signs into
//! the next resume token reads back unchanged.
#![no_main]

use libfuzzer_sys::fuzz_target;
use maidan_auth::subscribe::{
    resolve_subscribe, sign_resume_token, verify_resume_token, SubscribeFrame,
};

const SECRET: &[u8] = b"fuzz-subscribe-resume-secret-32b!!";

fuzz_target!(|text: &str| {
    let Ok(frame) = serde_json::from_str::<SubscribeFrame>(text) else {
        return;
    };
    let resumed = frame.resume_token.as_deref().is_some_and(|t| !t.is_empty());
    let Ok((filter, after_id)) = resolve_subscribe(&frame, Some(SECRET)) else {
        return;
    };
    assert!(!resumed, "a resume token the fuzzer wrote verified: {text}");
    assert!(after_id >= 0);
    assert!(after_id == 0 || filter.workspace_id.is_some());

    let Ok(token) = sign_resume_token(&filter, after_id, SECRET, 3600) else {
        return;
    };
    let (again, again_after) = verify_resume_token(&token, SECRET)
        .unwrap_or_else(|err| panic!("a token just signed fails: {err}"));
    assert_eq!(again_after, after_id);
    let fields = |f: &maidan_types::EventFilter| {
        (
            f.workspace_id,
            f.channel_id,
            f.thread_id,
            f.dm_conversation_id,
            f.member_id,
            f.kinds.clone(),
            f.channel_grants.clone(),
        )
    };
    assert_eq!(
        fields(&again),
        fields(&filter),
        "the filter from {text} does not survive its resume token"
    );
});
