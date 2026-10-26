### Cloudflare Turnstile on the waitlist and sign-up

- **Added:** Setting `MAIDAN_TURNSTILE_SECRET` and `MAIDAN_TURNSTILE_SITE_KEY` puts a Turnstile check on the waitlist form and on the sign-up start. The server verifies each token through the egress guard and checks the hostname and action. A missing or rejected token gets a 403, and a verification outage gets a 502, so nothing gets through when the check can't run. The console and landing page CSP allow `challenges.cloudflare.com` only while Turnstile is on. With the variables unset, nothing changes.
