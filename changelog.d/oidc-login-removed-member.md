### Sign-in re-checks the linked member

- **Fixed:** a sign-in whose OIDC identity is linked to a member that has since been removed (tombstoned or missing), or that belongs to another workspace, is now refused as not provisioned. Before, it resolved to the removed member. Auto-provisioning does not replace that member either (#1626).
