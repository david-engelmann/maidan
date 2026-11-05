### What a stranger meets on maidan.world

- **Added:** while the container is starting or a deploy replaces it, a person sees "Back in a moment", a page that retries itself and links the status page and support, and an API client gets a `503` `overloaded` problem with `Retry-After`. Both used to get the Containers runtime's plain-text 500.
- **Added:** a problem the server sends to a browser page, such as an expired invite or a sign-in with an address the invite wasn't for, is shown as a page with its title and detail, the request id to quote, and a support link, in place of raw JSON.
- **Added:** the landing page answers what it costs and where the data lives, and its footer links the docs, privacy, terms, support and status.
- **Changed:** the landing page, the README, the FAQ and Integration now say what works: Claude Code signs in with OAuth, and Cursor, Codex and other MCP clients connect with a token minted in the console. The Cursor and Claude Desktop examples point at maidan.world, and Claude Desktop's runs through `mcp-remote`, because its config file starts only local commands.
- **Changed:** the support and privacy pages say how an admin exports or erases a workspace: with an admin token, through the API. Hosted Operations has the invite email to send by hand.
- **Fixed:** the Worker passes `MAIDAN_EXPORT_SIGNING_KEY` and the Turnstile pair to the server, so maidan.world can sign exports and protect the waitlist once those secrets are set.
- **Changed:** maidan.world's free allowance, per workspace per month, is about $5 of what the service costs to run: 2 million requests, a million MCP calls, 200,000 embedding calls, 10,000 deliveries, an hour of sandboxed compute and 5 GiB stored, with media capped at the same 5 GiB.
