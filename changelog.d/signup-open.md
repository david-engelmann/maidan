### Open sign-up

- **Added:** `MAIDAN_SIGNUP_OPEN=1` lets any verified email sign up without an invite. Each sign-up gets its own new workspace, and the per-address and per-domain sign-up limits still apply. The audit row records `via: "open"`. Boot refuses the flag unless `MAIDAN_SIGNUP=1` is set, and refuses it beside `MAIDAN_SIGNUP_EMAIL_DOMAINS`.
