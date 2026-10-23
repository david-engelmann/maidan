### Members can set their own display name

- **Added:** `PATCH /members/{id}` sets or clears the caller's own display name (`workspace:write`). A display name could previously only be set at creation, so a member created without one stayed nameless in every surface that shows one.
