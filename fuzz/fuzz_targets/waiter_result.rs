//! A waiter's result envelope is whatever JSON an agent wrote with
//! `set_thread_result`; the result-delivery router parses every one.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) {
        let _ = maidan_types::waiter::parse_waiter_result(&value);
    }
});
