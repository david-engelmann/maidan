### Open sign-up needs a bot check in production
- **Fixed:** With `MAIDAN_ENV=production`, boot now refuses `MAIDAN_SIGNUP_OPEN=1` unless `MAIDAN_TURNSTILE_SECRET` is set, and the error names both variables. Outside production, open sign-up without Turnstile still boots but logs a warning.
