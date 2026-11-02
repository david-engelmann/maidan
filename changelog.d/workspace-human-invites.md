### Invite people to a workspace

- **Added:** A workspace admin invites a person by email as a viewer or member with `POST /workspaces/{wid}/invites`, lists invites, and revokes one. The link is shown once, works once, expires (7 days by default, 30 at most), is stored only as a hash, and never grants a role above the inviter's. The person signs in or signs up with `?invite=`, and joins only when the identity provider's verified email matches. The console's People page has an Invite form.
