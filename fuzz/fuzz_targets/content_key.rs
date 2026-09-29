//! The content KEK is parsed from operator configuration, as hex or base64. A
//! key that parses from hex parses back from its own hex.
#![no_main]

use libfuzzer_sys::fuzz_target;
use maidan_types::content_seal::parse_key_32;

fuzz_target!(|raw: &str| {
    let Ok(key) = parse_key_32(raw) else {
        return;
    };
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(parse_key_32(&hex).ok(), Some(key));
});
