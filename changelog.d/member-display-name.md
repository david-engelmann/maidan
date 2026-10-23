### Members can set their own display name

- **Added:** `PATCH /members/{id}` sets or clears the caller's own display name (`workspace:write`). A display name could previously only be set at creation, so a member created without one stayed nameless in every surface that shows one.
- **Changed:** a display name is now checked the same way on every path that sets one (`PATCH /members/{id}`, member create, agent create): trimmed, blank means none, and over 100 characters or containing a control character (NUL, newlines) or an invisible format character (zero-width, bidi overrides and isolates) is a 400.
