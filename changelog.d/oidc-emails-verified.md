### An OIDC provider can vouch for its emails

- **Added:** `MAIDAN_OIDC_EMAILS_VERIFIED=1` treats an `email` claim that arrives without `email_verified` as verified, for a provider whose every login proves the address (Cloudflare Access with its emailed one-time code). An explicit `email_verified: false` still counts as unverified. maidan.world signs in through Cloudflare Access, whose tokens do not document the claim.
