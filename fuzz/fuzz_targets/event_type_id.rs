//! `$type` ids and projector type lists come from clients. A recognised id is
//! exactly the one its kind prints.
#![no_main]

use libfuzzer_sys::fuzz_target;
use maidan_types::EventKind;

fuzz_target!(|raw: &str| {
    if let Some(kind) = EventKind::parse_type_id(raw) {
        assert_eq!(kind.type_id(), raw);
    }
    let _ = maidan_types::cursor::parse_projector_types(raw);
});
