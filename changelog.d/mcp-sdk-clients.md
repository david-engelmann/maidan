### The official MCP SDKs as clients in CI, and connect recipes

- **Added:** A report-only CI job, `mcp sdk clients`, runs the official
  TypeScript (`@modelcontextprotocol/sdk`) and Python (`mcp`) SDKs as clients
  of a seeded server on `/mcp/streamable` and `/mcp`: initialize, list tools,
  post and read back a message, and check that a bad argument is refused by
  name. `scripts/mcp-sdk-clients.sh` runs the same thing locally.
- **Added:** `examples/connect/` has recipes for claude.ai custom connectors,
  Perplexity, grok.com and the Gemini app, with a `MAIDAN_OAUTH_CLIENTS`
  example. Only the claude.ai flow has been run end to end; the others say so.
- **Changed:** docs/Clients.md's claude.ai section describes the OAuth flow
  instead of the no-sign-in dev instance.
