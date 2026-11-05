### A Helm install boots with sign-in on

- **Fixed:** the `maidan` chart mounts `MAIDAN_SESSION_SECRET` from `existingSecret` by default. The default key list held only `DATABASE_URL` and `MAIDAN_CONTENT_KEK`, so an install that followed the docs gave a server with sign-in on no session secret, and it refused to boot.
- **Changed:** a production render without `existingSecret` refuses when no session secret is supplied (`secrets.MAIDAN_SESSION_SECRET`, a `secretFiles.extra` source or `extraEnvFrom`), unless `config.AUTH_DISABLED` is set, instead of shipping a pod that never starts.
