//! The SSRF guard on every operator-supplied URL. Whatever it accepts must
//! still be accepted, unchanged, when its own serialized form is checked
//! again: a URL that normalizes into something the guard would refuse is how
//! a parser differential turns into a request to an internal address.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|raw: &str| {
    let Ok(url) = maidan_auth::parse_egress_target(raw) else {
        return;
    };
    assert!(matches!(url.scheme(), "http" | "https"));
    assert!(url.username().is_empty() && url.password().is_none());
    let again = maidan_auth::parse_egress_target(url.as_str())
        .unwrap_or_else(|err| panic!("{raw:?} passed but its form {url} fails: {err:?}"));
    assert_eq!(again, url, "{raw:?} is not stable under re-checking");
});
