### Webhooks and workspace secrets on the hosted service

- **Fixed:** the Cloudflare Worker now passes `FEDERATION_ENCRYPTION_KEY` to the server. Without it, creating a webhook, a federation peer, an A2A push config or a workspace secret on maidan.world answered 500.
