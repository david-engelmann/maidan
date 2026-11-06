### Changed

The Keycloak example realms now pin short-lived access tokens (5 minutes), rotate refresh tokens, keep PKCE S256 on public clients, and refuse implicit, password, registration, and wildcard redirect postures; a contract test locks the policy in.
