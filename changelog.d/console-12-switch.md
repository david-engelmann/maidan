### The console replaces the old board at /ui

- **Changed:** `/ui` now serves the console built from `web/console`; `/console` and `/console/` answer 308 to `/ui/`. Its assets are served at `/ui/{name}`, and the service worker at `/ui/sw.js` takes over the old board's registration.
- **Added:** the console has its own sign-in when signed out: **Sign in** through the instance's sign-in page (`/auth/oidc/login?return_to=/ui/`), or paste a token, which is traded for a session and never stored.
- **Removed:** the old board (`static/index.html`, `static/ui`), `scripts/gen-ui-client.mjs`, the Rust tests that pinned its JavaScript and CSS, and the CI steps that fetched TypeScript for them. The OAuth consent pages keep their CSP, now named `OAUTH_PAGE_CSP`.
- **Changed:** `ui-tests` drives the console: a new `console.spec.ts` (CSP, the redirect, sign-in, the board, needs you) and the kept `oauth-consent.spec.ts`; the old board's specs are deleted. `npm run capture` shoots the console, and the old board's connect, error, refused-close and thread screenshots are removed.
