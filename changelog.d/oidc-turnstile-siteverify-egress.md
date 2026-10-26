### Turnstile docs name the siteverify egress it needs

- **Fixed:** docs/OIDC.md now says the server calls Cloudflare's siteverify at `challenges.cloudflare.com` through the egress guard, so outbound HTTPS to that host must be allowed and it must resolve to a public address. Otherwise every waitlist join and sign-up start gets a 502, and the log line to look for is named (#1633).
