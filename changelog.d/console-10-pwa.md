### Console as an installable app

- **Added:** A web app manifest and an install button under Tools → This device, so the console can be added to a home screen or dock.
- **Added:** A service worker at `/console/sw.js` that keeps only the static console shell for offline starts. API, sign-in, socket, query-string and token-bearing requests always go to the network and are never cached.
- **Added:** Web Push from the console: turn notifications on or off for this device, using the server's existing VAPID key and push subscription endpoints. Clicking a notification opens Needs you.
