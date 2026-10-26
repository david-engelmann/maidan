### The waitlist page links the instance's privacy policy and terms

- **Added:** `MAIDAN_SIGNUP_PRIVACY_URL` and `MAIDAN_SIGNUP_TERMS_URL` name where an instance's privacy policy and terms live, as a path on its origin or an `https://` URL. The waitlist page at `/` links each one that is set in its footer, since it asks visitors for an email address. Boot refuses any other shape, and any value with a quote, angle bracket, space or control character.
- **Changed:** maidan.world's Worker no longer answers `/`. The server's waitlist page serves it, and the Worker keeps only `/privacy`, `/terms` and `/support`, which `wrangler.prod.jsonc` now links from that page.
