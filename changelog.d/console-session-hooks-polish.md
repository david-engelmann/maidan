### A sign-in stays in the workspace it belongs to

- **Fixed:** Switching tokens, signing out, or an expired session drops the previous workspace, its grant, its presence, and the board, roster, and footprint that belonged to it. A confirmation secret is never sent as the sign-in return path, and a bad token's error clears as you edit it.
- **Added:** The board remembers the open channel per workspace, and Sign in can return to the console page you were on.
