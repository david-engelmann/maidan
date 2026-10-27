### Email linking is scoped to the configured issuer

- **Fixed:** with `MAIDAN_OIDC_LINK_EMAIL` on, a first sign-in links to an existing member by verified email only when the configured OIDC issuer asserted that email. An email verified by any other issuer no longer links to the member (#1625).
