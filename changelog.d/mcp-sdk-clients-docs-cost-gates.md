### MCP SDK client bounds and docs Worker cost

- **Fixed:** `mcp-sdk-clients.sh` refuses out-of-range ports and unknown client names before boot.
- **Fixed:** The TypeScript and Python MCP SDK clients validate loopback/https bases, allowlisted `/mcp` endpoints, and token shape before importing their SDKs.
- **Fixed:** `cost.test.js` locks `wrangler.docs.jsonc` to static assets with no container and no wake cron.
