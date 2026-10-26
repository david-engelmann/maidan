### SCIM delete removes the member
- **Fixed:** A SCIM delete now tombstones the member and deletes its OIDC identities in the same transaction that revokes its tokens and ends its sessions. Before this, the member stayed live with its identities attached, so a user deleted through SCIM could sign straight back in.
