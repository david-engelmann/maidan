### Email linking claims a member on its first sign-in only

- **Security:** with `MAIDAN_OIDC_LINK_EMAIL=1`, a verified email links only to a member that no other identity has signed in as. A handle equal to an email stays the pre-provisioning key, and SCIM-provisioned members still link, but a member someone already signs in as is never handed to a second identity asserting the same address (SEC-007).
