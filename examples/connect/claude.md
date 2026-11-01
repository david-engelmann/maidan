# claude.ai custom connector

claude.ai identifies itself with a client metadata document that Anthropic
hosts on `claude.ai`, so nothing is pre-registered. Maidan's authorization
server metadata advertises `client_id_metadata_document_supported: true` and
`none` among its token endpoint auth methods, which is what Claude checks
before it uses a metadata document
([Claude's connector authentication docs](https://claude.com/docs/connectors/building/authentication)).

## Operator

1. Set `MAIDAN_PUBLIC_ORIGIN` to the instance's public HTTPS origin.
2. Set `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS=claude.ai`. The hosted instances
   already do. Any other host's document is refused before it is fetched.
3. Configure a sign-in method (OIDC) so a signed-out browser can sign in.
4. If a firewall fronts the instance, allow Anthropic's published egress range
   `160.79.104.0/21`; Anthropic's servers make the MCP requests.

## Person

1. In claude.ai, open the connector settings and add a custom connector.
2. Give it the URL `https://<your instance>/mcp/streamable`. Leave the OAuth
   client id and secret empty.
3. Connect. The browser goes to Maidan; sign in if asked, then approve the
   scopes on Maidan's consent page.
4. In a chat, ask Claude to call `whoami`. It should name your member.

To disconnect, remove the connector in claude.ai; an operator can also revoke
the grant in Maidan.

## On the hosted room

At `maidan.world` the URL is `https://maidan.world/mcp/streamable`, and the
instance settings above are already in place. Hosted sign-up is by waitlist
invite.
