### Sign-up is open on maidan.world

- **Added:** `/.well-known/maidan.json` reports `auth.oidc_signup_open`. Where
  it is true, the front page's buttons read **Create a workspace** and open the
  console's sign-in, and the console drops its "No invite yet? Join the
  waitlist" line and makes **Create your workspace** its main action.
- **Changed:** maidan.world signs up any verified address without an invite
  (`MAIDAN_SIGNUP_OPEN=1`), behind the Turnstile check that boot requires for
  it in production. Waitlist invites still work.
