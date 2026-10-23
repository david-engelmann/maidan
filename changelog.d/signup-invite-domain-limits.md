### Sign-up and invite charged per email domain

- **Added:** an invite mint and a finished sign-up are each charged to the email's domain (`MAIDAN_SIGNUP_DOMAIN_RATE_LIMIT_MAX` and `MAIDAN_SIGNUP_DOMAIN_RATE_LIMIT_WINDOW_SECS`, default 100 per hour, in separate buckets for invites and sign-ups), on top of the per-address budget the launch surface already has. Until now only a waitlist join was charged per domain. A refused mint or sign-up changes nothing, and two addresses share one domain's budget.
