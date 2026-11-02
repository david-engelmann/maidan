### A SCIM-deactivated member's tokens stop resolving at the lookup

- **Fixed:** the bearer-token lookup refuses a token whose member, or whose delegate, is deactivated over SCIM (`active = false`), on both backends, as browser sessions already did. Before, it relied on the SCIM update revoking the tokens in the same transaction, so a token that survived the revoke kept working (#1706).
