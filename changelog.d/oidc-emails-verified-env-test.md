### OIDC: a unit test for MAIDAN_OIDC_EMAILS_VERIFIED

- **Fixed:** `OidcSettings::from_env` now has a test that `MAIDAN_OIDC_EMAILS_VERIFIED` is off by default, on for `1`/`true`/`TRUE`, and off for anything else.
