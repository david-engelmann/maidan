### Sign-up: one workspace per identity under concurrent callbacks

- **Fixed:** two sign-up callbacks for the same identity at the same time could each create a workspace. The sign-up transaction now runs the existing-identity check again under a lock (a Postgres advisory lock on the issuer and subject; `BEGIN IMMEDIATE` on SQLite), so the second callback is told to sign in instead.
