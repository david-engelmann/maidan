### Sign-up start keeps the Turnstile token out of the URL

- **Fixed:** The console's sign-up check now POSTs its Turnstile token to `/auth/oidc/signup` in a form body instead of putting it in the query string, so the token no longer reaches proxy or access logs or the browser history. `GET /auth/oidc/signup` no longer reads a `turnstile` query parameter (#1632).
