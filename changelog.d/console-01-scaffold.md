### The React console starts at /console, beside the board

- **Added:** `web/console`, a React 19 + Vite + Motion app in the landing's steel look, built into the binary like the landing and served at `/console` under its own CSP (`CONSOLE_CSP`: external scripts and stylesheets only, plus `style-src-attr 'unsafe-inline'` for motion). The board at `/ui` is unchanged and still served.
- **Added:** the console build fails when the built `index.html` has an inline script, a `<style>` element, an inline handler or a `javascript:` URL (`web/console/scripts/check-csp.mjs`, with tests). CI runs the console typecheck, lint, tests and build.

- **Changed:** The built page preloads every chunk `app.js` imports (`<link rel="modulepreload">`, no polyfill), and the CSP-checked build fails if one is missing.
