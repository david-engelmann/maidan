//! The content keyring from configuration, shared by the server and the CLI.
//! The design is in `docs/Decisions.md` ("Crypto-shredding of message content"
//! and "The content KEK fails closed").

use std::sync::Arc;

use maidan_types::{parse_key_32, ContentKeyring, LOCAL_CUSTODY};

use crate::kms_custody::{self, AwsKmsCustody};

/// Primary key-encryption key: 32 bytes, base64 or 64 hex characters.
pub const KEK_ENV: &str = "MAIDAN_CONTENT_KEK";
/// Comma-separated previous KEKs, kept to unwrap keys until rewrapped.
pub const PREVIOUS_KEKS_ENV: &str = "MAIDAN_CONTENT_KEK_PREVIOUS";
/// Explicit development opt-in: with no KEK set, wrap keys with the public
/// development KEK instead of refusing to start. Rejected in production.
pub const ALLOW_DEV_KEK_ENV: &str = "MAIDAN_ALLOW_INSECURE_DEV_KEK";

/// Who holds workspace keys: `local` (the instance KEK, the default) or
/// `aws-kms` (a KMS key per workspace key).
pub const CUSTODY_ENV: &str = "MAIDAN_CONTENT_KEY_CUSTODY";
/// An AWS KMS endpoint other than the region's, as for a test or LocalStack.
pub const KMS_ENDPOINT_ENV: &str = "MAIDAN_KMS_ENDPOINT";
/// Days KMS waits before it deletes a destroyed workspace's key (7 to 30;
/// default 7).
pub const KMS_DELETION_DAYS_ENV: &str = "MAIDAN_KMS_KEY_DELETION_DAYS";

/// The KEK settings as read from the environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct KekConfig<'a> {
    pub primary: Option<&'a str>,
    pub previous: Option<&'a str>,
    /// `MAIDAN_ENV=production`.
    pub production: bool,
    /// `MAIDAN_ALLOW_INSECURE_DEV_KEK=1`.
    pub allow_dev_kek: bool,
}

/// Build the keyring from `MAIDAN_CONTENT_KEK`, `MAIDAN_CONTENT_KEK_PREVIOUS`
/// and `MAIDAN_ALLOW_INSECURE_DEV_KEK`, with the workspace-key custody
/// `MAIDAN_CONTENT_KEY_CUSTODY` names. The instance KEK stays required under a
/// custody: it opens the workspace keys made before the custody was chosen.
pub async fn from_env(production: bool) -> Result<ContentKeyring, String> {
    let keyring = kek_from_env(production)?;
    let custody = std::env::var(CUSTODY_ENV).ok();
    match custody.as_deref().map(str::trim) {
        None | Some("") => Ok(keyring),
        Some(name) if name == LOCAL_CUSTODY => Ok(keyring),
        Some(kms_custody::NAME) => {
            let custody = aws_kms_from_env().await?;
            tracing::info!(
                custody = kms_custody::NAME,
                "workspace keys are held in AWS KMS"
            );
            Ok(keyring.with_custody(Arc::new(custody)))
        }
        Some(other) => Err(format!(
            "{CUSTODY_ENV}: `{other}` is not a key custody; use `{LOCAL_CUSTODY}` or `{}`",
            kms_custody::NAME
        )),
    }
}

/// The AWS KMS custody, on the SDK's usual credential and region chain.
async fn aws_kms_from_env() -> Result<AwsKmsCustody, String> {
    let deletion_days = match std::env::var(KMS_DELETION_DAYS_ENV).ok() {
        None => *kms_custody::DELETION_DAYS.start(),
        Some(raw) => raw
            .trim()
            .parse::<i32>()
            .ok()
            .filter(|days| kms_custody::DELETION_DAYS.contains(days))
            .ok_or_else(|| {
                format!("{KMS_DELETION_DAYS_ENV}: a whole number of days from 7 to 30")
            })?,
    };
    let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    if shared.region().is_none() {
        return Err(format!(
            "{CUSTODY_ENV}={}: no AWS region is set (AWS_REGION)",
            kms_custody::NAME
        ));
    }
    let mut config = aws_sdk_kms::config::Builder::from(&shared);
    if let Some(endpoint) = std::env::var(KMS_ENDPOINT_ENV)
        .ok()
        .filter(|endpoint| !endpoint.trim().is_empty())
    {
        config = config.endpoint_url(endpoint.trim());
    }
    Ok(AwsKmsCustody::new(
        aws_sdk_kms::Client::from_conf(config.build()),
        deletion_days,
    ))
}

fn kek_from_env(production: bool) -> Result<ContentKeyring, String> {
    let primary = std::env::var(KEK_ENV).ok();
    let previous = std::env::var(PREVIOUS_KEKS_ENV).ok();
    from_config(KekConfig {
        primary: primary.as_deref(),
        previous: previous.as_deref(),
        production,
        allow_dev_kek: matches!(
            std::env::var(ALLOW_DEV_KEK_ENV).as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ),
    })
}

/// A KEK is required. The public development KEK is used only when no KEK is
/// set and the development opt-in is, outside production, so a publicly known
/// key never protects data because a variable was forgotten. A malformed key
/// is always an error: dropping an old key would strand every data key it
/// wraps.
pub fn from_config(config: KekConfig<'_>) -> Result<ContentKeyring, String> {
    if config.production && config.allow_dev_kek {
        return Err(format!(
            "{ALLOW_DEV_KEK_ENV} is not allowed when MAIDAN_ENV=production"
        ));
    }
    let primary = config.primary.map(str::trim).filter(|raw| !raw.is_empty());
    let previous = config
        .previous
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
        None if !previous.is_empty() => Err(format!(
            "{PREVIOUS_KEKS_ENV} is set without {KEK_ENV}; set the new primary KEK"
        )),
        None if config.allow_dev_kek => {
            tracing::warn!(
                "{KEK_ENV} not set and {ALLOW_DEV_KEK_ENV}=1: message content keys are wrapped \
                 with the public development KEK; never use this for real data"
            );
            Ok(ContentKeyring::insecure_dev())
        }
        None => Err(format!(
            "{KEK_ENV} must be set: it wraps the keys that make withdrawn messages unreadable \
             (generate one with `openssl rand -hex 32`). For local development only, \
             {ALLOW_DEV_KEK_ENV}=1 uses a public development key instead"
        )),
    }
}

const INVALID: &str = "each key must decode to 32 bytes (base64 or 64 hex characters)";

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0101010101010101010101010101010101010101010101010101010101010101";

    fn config(primary: Option<&'static str>) -> KekConfig<'static> {
        KekConfig {
            primary,
            ..KekConfig::default()
        }
    }

    #[test]
    fn a_kek_is_required_without_the_dev_opt_in() {
        let err = from_config(config(None)).unwrap_err();
        assert!(
            err.contains(KEK_ENV) && err.contains(ALLOW_DEV_KEK_ENV),
            "{err}"
        );
        let err = from_config(KekConfig {
            production: true,
            ..config(None)
        })
        .unwrap_err();
        assert!(err.contains(KEK_ENV), "{err}");
        assert!(from_config(config(Some("  "))).is_err());
        assert!(from_config(KekConfig {
            production: true,
            ..config(Some(HEX))
        })
        .is_ok());
    }

    #[test]
    fn the_dev_opt_in_uses_the_public_kek_only_without_a_kek() {
        let dev = KekConfig {
            allow_dev_kek: true,
            ..config(None)
        };
        assert!(from_config(dev).unwrap().is_insecure_dev());
        let set = KekConfig {
            allow_dev_kek: true,
            ..config(Some(HEX))
        };
        assert!(!from_config(set).unwrap().is_insecure_dev());
    }

    #[test]
    fn production_rejects_the_dev_opt_in_even_with_a_kek() {
        for primary in [None, Some(HEX)] {
            let err = from_config(KekConfig {
                primary,
                production: true,
                allow_dev_kek: true,
                ..KekConfig::default()
            })
            .unwrap_err();
            assert!(err.contains(ALLOW_DEV_KEK_ENV), "{err}");
        }
    }

    #[test]
    fn a_malformed_key_is_an_error_not_a_skip() {
        assert!(from_config(config(Some("short"))).is_err());
        let previous = format!("{HEX}, nope");
        let err = from_config(KekConfig {
            previous: Some(&previous),
            ..config(Some(HEX))
        })
        .unwrap_err();
        assert!(err.contains(PREVIOUS_KEKS_ENV), "{err}");
    }

    #[test]
    fn previous_keks_without_a_primary_are_refused() {
        let err = from_config(KekConfig {
            previous: Some(HEX),
            allow_dev_kek: true,
            ..KekConfig::default()
        })
        .unwrap_err();
        assert!(err.contains(PREVIOUS_KEKS_ENV), "{err}");
    }

    #[test]
    fn a_rotated_keyring_unwraps_keys_from_the_previous_kek() {
        let old = from_config(config(Some(HEX))).unwrap();
        let subject = uuid::Uuid::now_v7();
        let key = maidan_types::ContentKey::generate();
        let wrapped = old.wrap(subject, &key).unwrap();
        let new_hex = "02".repeat(32);
        let rotated = from_config(KekConfig {
            primary: Some(&new_hex),
            previous: Some(HEX),
            ..KekConfig::default()
        })
        .unwrap();
        assert_ne!(rotated.primary_id(), wrapped.kek_id);
        assert_eq!(rotated.unwrap(subject, &wrapped).unwrap(), key);
    }
}
