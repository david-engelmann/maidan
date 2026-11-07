### CI jobs hold only the permissions they use

- **Security:** The docs workflow grants `pages: write` and `id-token: write` to its GitHub Pages deploy job alone, not to the build that runs repository scripts. Every checkout in the CI and nightly workflows sets `persist-credentials: false`, since none of their jobs pushes.
- **Security:** The Helm chart's server pod sets `automountServiceAccountToken: false`: the server never calls the Kubernetes API, so no service-account token is mounted for a compromised process to use. The stack's vendored chart is repackaged.
- **Changed:** The optional Intel Mac release build runs on `macos-15-intel`; GitHub retired `macos-13`.
