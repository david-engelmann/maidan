# Maidan on Claude Desktop (local)

This is the primary local test path: Claude Desktop runs on your Mac, so it
reaches `localhost` directly. No tunnel, no public URL, no Cloudflare.

## One-time setup

```bash
cd ~/workspace/maidan-connected-app-harness/tools

# Self-signed cert for localhost (trust it in Keychain once).
openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem \
  -days 365 -nodes -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
```

## Every test session

Terminal 1 — Maidan server (HTTP on 8080, no-auth):
```bash
DATABASE_URL="sqlite:/tmp/maidan-local/maidan.db" \
MAIDAN_BIND="127.0.0.1:8080" \
MAIDAN_ALLOW_INSECURE_DEV_KEK=1 \
MAIDAN_SESSION_SECRET="dev-only-local-test-secret" \
MAIDAN_ALLOW_INSECURE_NO_AUTH=1 \
AUTH_DISABLED=1 \
~/workspace/maidan-harness-server-target/debug/maidan-server \
  >> /tmp/maidan-harness-server.log 2>&1
```

Terminal 2 — HTTPS proxy (Claude Desktop requires `https://`):
```bash
cd ~/workspace/maidan-connected-app-harness/tools
python3 https_proxy.py --cert cert.pem --key key.pem \
  --listen-port 18001 --target-port 8080
```

## Add the connector in Claude Desktop

Settings → Connectors → Add custom connector:
- Name: `maidan-local`
- MCP server URL: `https://localhost:18001/mcp`
- Auth: **No sign-in** (the server is in no-auth mode)

Claude Desktop will list the tools (240 of them). The harness's
`search_messages`, `post_message`, and `request_approval` suites map to
real tools on this server.

## Verify it works

In a Claude Desktop chat with the connector enabled:
> "Search Maidan for the word onboarding and summarize what you find."

Then check the server log for the tool call:
```bash
grep "search_messages" /tmp/maidan-harness-server.log | tail -3
```

## Notes

- This is the same pattern as the existing `dusk-local` connector
  (`https://localhost:18001/mcp`) — pick a different port (e.g. 18002)
  if both run at once.
- The harness's browser providers target claude.ai web; Claude Desktop
  is a manual surface for now (same tier as Cursor/Meta in the harness).
- Never use the no-auth server or the self-signed cert beyond local
  testing.
