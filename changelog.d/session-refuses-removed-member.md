### Sessions refuse members removed over SCIM

- **Fixed:** a sign-in that was already in progress when a SCIM delete or deactivation committed could still create a live session for the removed member. Creating a session now checks, in the same write, that the member is neither tombstoned nor SCIM-inactive, and an existing session for such a member no longer resolves.
