### OAuth client metadata hosts are an instance setting, and signed-out browsers sign in from authorize

- **Changed:** The hosts an OAuth client metadata document may be fetched from are now set for the instance with `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS` (comma-separated exact host names, empty by default) instead of being read from each workspace's egress targets. A workspace fresh from sign-up can now connect an MCP client that identifies itself by a metadata document. Operators who relied on egress rows for this must list those hosts in the new setting.
- **Changed:** A signed-out browser at `GET /oauth/authorize` is redirected to sign in and brought back to the same request, instead of getting a JSON 401. A request with a bad bearer still gets a 401.
- **Fixed:** A sign-in `return_to` with a backslash or control character now falls back to the console, like `//host` and absolute URLs already did.
