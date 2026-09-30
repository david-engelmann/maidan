//! A2A list calls hand out opaque `pageToken`s, and a client can send back any
//! string. Whatever decodes is a position the server can hand out again: it
//! encodes to a token that decodes to the same position.
#![no_main]

use chrono::{DurationRound, TimeDelta};
use libfuzzer_sys::fuzz_target;
use maidan_a2a::page_token::{
    decode_config_cursor, decode_task_cursor, encode_config_cursor, encode_task_cursor,
};

fuzz_target!(|token: &str| {
    if let Ok(config_id) = decode_config_cursor(token) {
        assert_eq!(encode_config_cursor(&config_id), token, "not canonical");
    }
    if let Ok((at, id)) = decode_task_cursor(token) {
        // The listing sorts to the millisecond, so that is what a token keeps.
        let Ok(at) = at.duration_trunc(TimeDelta::milliseconds(1)) else {
            return;
        };
        let cursor = (at, id);
        let printed = encode_task_cursor(&cursor);
        assert_eq!(
            decode_task_cursor(&printed).ok().as_ref(),
            Some(&cursor),
            "{token:?} decoded to {cursor:?}, which prints as {printed:?}"
        );
    }
});
