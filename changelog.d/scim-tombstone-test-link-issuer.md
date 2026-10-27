### Main builds again after the OIDC link-issuer change

- **Fixed:** The SCIM-delete tombstone test now passes the configured issuer as `link_email_issuer`, matching the signature from #1653. It was merged in #1663 with the old boolean and broke the build.
