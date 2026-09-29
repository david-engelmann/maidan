//! `maidan://` room URIs arrive from clients and peers. A parsed URI prints
//! back to a string that parses to the same URI.
#![no_main]

use libfuzzer_sys::fuzz_target;
use maidan_types::RoomUri;

fuzz_target!(|raw: &str| {
    let Ok(uri) = RoomUri::parse(raw) else {
        return;
    };
    let printed = uri.to_string();
    assert_eq!(
        RoomUri::parse(&printed).as_ref(),
        Ok(&uri),
        "{raw:?} printed as {printed:?}"
    );
});
