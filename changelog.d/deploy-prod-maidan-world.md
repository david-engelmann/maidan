### maidan.world runs the newest release on Cloudflare

- **Added:** `deploy prod` deploys a published release to maidan.world: it verifies the release image's cosign signature, pushes it to the account registry, deploys `deploy/cloudflare/wrangler.prod.jsonc`, waits for `/health` to report the tag, and checks that an anonymous `/me` is refused. Sign-in is a Cloudflare Access for SaaS OIDC app, and sign-up is by waitlist invite.
- **Added:** the Worker in front of both instances redirects plain HTTP to HTTPS, sends a browser opening `/` to the console, and adds HSTS, `X-Content-Type-Options: nosniff` and a referrer policy to every answer, leaving WebSocket upgrades untouched.
- **Added:** maidan.world serves its own landing page, privacy policy, terms of service and support page from the Worker's static assets (`deploy/cloudflare/site/`), with a strict content security policy; self-hosted instances and dev are unchanged.
