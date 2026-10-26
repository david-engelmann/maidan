### Sign-up start checks Turnstile before the invite

- **Fixed:** `GET /auth/oidc/signup` now runs the Turnstile check before it looks up the invite. A caller without a valid token gets the same refusal for an unknown, expired or valid invite, and a browser is still sent to the console check (#1631).
