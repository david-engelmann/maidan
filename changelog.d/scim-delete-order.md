### SCIM delete ends sessions after a racing sign-in

- **Fixed:** SCIM delete now removes the SCIM row before ending the member's tokens and sessions, so it waits behind an in-flight sign-in and also ends the session that sign-in commits, rather than leaving an unusable orphan session row for the tombstoned member (#1716).
