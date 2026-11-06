### A workspace can be named when it is created at sign-up

- **Added:** Sign-up takes an optional workspace name, on the console form and on `GET` and `POST /auth/oidc/signup`. Left blank, the workspace is still named after the person. A name longer than 200 characters, or one with a control character, is refused. A rename uses that same control-character rule.
