### claude.ai's connector can sign in: an omitted OAuth resource means the MCP endpoint

- **Fixed:** `/oauth/authorize` and `/oauth/token` refused a request that named no `resource` with `invalid_target`, and claude.ai's custom connector sends none, so every claude.ai sign-in stopped on a Bad Request page. A request without a resource now gets the one resource the server issues tokens for (`<origin>/mcp/streamable`), and the grant and its tokens are bound to it as before. A request that names any other resource is still refused.
