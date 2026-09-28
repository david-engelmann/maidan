//! The content keyring from configuration, shared by the server and the CLI.
//! The design is in `docs/Decisions.md` ("Crypto-shredding of message content").

use maidan_types::{parse_key_32, ContentKeyring};

/// Primary key-encryption key: 32 bytes, base64 or 64 hex characters.
pub const KEK_ENV: &str = "MAIDAN_CONTENT_KEK";
/// Comma-separated previous KEKs, kept to unwrap keys until rewrapped.
pub const PREVIOUS_KEKS_ENV: &str = "MAIDAN_CONTENT_KEK_PREVIOUS";

/// Build the keyring from `MAIDAN_CONTENT_KEK` / `MAIDAN_CONTENT_KEK_PREVIOUS`.
pub fn from_env(production: bool) -> Result<ContentKeyring, String> {
    from_config(
        std::env::var(KEK_ENV).ok().as_deref(),
        std::env::var(PREVIOUS_KEKS_ENV).ok().as_deref(),
        production,
    )
}

/// Production requires a primary KEK. Elsewhere an unset one falls back to the
/// public development keyring, with a warning. A malformed key is always an
/// error: dropping an old key would strand every data key it wraps.
pub fn from_config(
    primary: Option<&str>,
    previous: Option<&str>,
    production: bool,
) -> Result<ContentKeyring, String> {
    let primary = primary.map(str::trim).filter(|raw| !raw.is_empty());
    let previous = previous
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| parse_key_32(raw).map_err(|_| format!("{PREVIOUS_KEKS_ENV}: {INVALID}")))
        .collect::<Result<Vec<_>, _>>()?;
    match primary {
        Some(raw) => {
            let primary = parse_key_32(raw).map_err(|_| format!("{KEK_ENV}: {INVALID}"))?;
            Ok(ContentKeyring::new(primary, previous))
        }
        None if production => Err(format!(
            "{KEK_ENV} must be set in production: it wraps the keys that make withdrawn messages unreadable"
        )),
        None if !previous.is_empty() => Err(format!(
            "{PREVIOUS_KEKS_ENV} is set without {KEK_ENV}; set the new primary KEK"
        )),
        None => {
            tracing::warn!(
                "{KEK_ENV} not set; message content keys are wrapped with the public development KEK"
            );
            Ok(ContentKeyring::insecure_dev())
        }
    }
}

const INVALID: &str = "each key must decode to 32 bytes (base64 or 64 hex characters)";

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0101010101010101010101010101010101010101010101010101010101010101";

    #[test]
    fn production_requires_a_primary_kek() {
        let err = from_config(None, None, true).unwrap_err();
        assert!(err.contains(KEK_ENV), "{err}");
        assert!(from_config(Some(HEX), None, true).is_ok());
    }

    #[test]
    fn development_falls_back_to_the_public_kek() {
        assert!(from_config(None, None, false).unwrap().is_insecure_dev());
        assert!(from_config(Some("  "), None, false)
            .unwrap()
            .is_insecure_dev());
        assert!(!from_config(Some(HEX), None, false)
            .unwrap()
            .is_insecure_dev());
    }

    #[test]
    fn a_malformed_key_is_an_error_not_a_skip() {
        assert!(from_config(Some("short"), None, false).is_err());
        let err = from_config(Some(HEX), Some(&format!("{HEX}, nope")), true).unwrap_err();
        assert!(err.contains(PREVIOUS_KEKS_ENV), "{err}");
    }

    #[test]
    fn previous_keks_without_a_primary_are_refused() {
        assert!(from_config(None, Some(HEX), false).is_err());
    }

    #[test]
    fn a_rotated_keyring_unwraps_keys_from_the_previous_kek() {
        let old = from_config(Some(HEX), None, true).unwrap();
        let subject = uuid::Uuid::now_v7();
        let key = maidan_types::ContentKey::generate();
        let wrapped = old.wrap(subject, &key).unwrap();
        let new_hex = "02".repeat(32);
        let rotated = from_config(Some(&new_hex), Some(HEX), true).unwrap();
        assert_ne!(rotated.primary_id(), wrapped.kek_id);
        assert_eq!(rotated.unwrap(subject, &wrapped).unwrap(), key);
    }
}
