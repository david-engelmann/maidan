### Console and landing fixes from the live audit

- **Fixed:** the signed-out sign-in card's "Sign in" and "Create a workspace" buttons span the card like the token field, instead of sitting in a narrow column on the right.
- **Fixed:** after a waitlist request, the landing sheet's "You're on the list." replaces the "Take a place on the board." heading and its intro, instead of rendering under them.
- **Fixed:** the board's presence chips no longer print ", online, free" beside the name with a space before the comma: the status is hidden for screen readers with the class the stylesheet defines (`sr-only` was never defined), and reads as one sentence. A vote chip's voter names, hidden the same broken way, are fixed too.
- **Fixed:** the Peers cards (Tools, Workspace and Explore) say "Needs a pasted token with federation:admin." as a quiet note when the credential lacks it, as a browser session always does, instead of a red error with Try again on every load. A failed add or remove still reports.
- **Fixed:** a console hash that names no place (`#/files`) shows a not-found state with a link to the board, instead of silently opening the board.
- **Changed:** `docs/Production.md` says the Zero Trust account behind maidan.world sign-in needs an Access App Launcher application, or the one-time-PIN login never returns to maidan.world.
