### OIDC identity row decides racing first logins

- **Fixed:** Two simultaneous first logins of the same OIDC identity could bind it to different members, the later write silently moving the identity. The identity row now keeps its first member, and a login that loses the race signs in as the member that won (#1642).
