# SDK release process

The four client SDKs (`sdk/{typescript,python,go,rust}`) are versioned and
published **independently of the server**, by pushing a per-language tag. The
[`sdk-release`](../.github/workflows/sdk-release.yml) workflow does the publish;
the server's `release.yml` (on `vX.Y.Z` tags) is untouched.

## Tags → registries

| Push this tag | Publishes | From | Registry |
|---------------|-----------|------|----------|
| `sdk-ts-vX.Y.Z` | TypeScript | `sdk/typescript` | npm (`npm publish`) |
| `sdk-py-vX.Y.Z` | Python | `sdk/python` | PyPI (`twine upload`) |
| `sdk-rs-vX.Y.Z` | Rust | `sdk/rust` | crates.io (`cargo publish`) |
| `sdk-go-vX.Y.Z` | Go | `sdk/go` | *(no registry)* — the job re-tags the commit as **`sdk/go/vX.Y.Z`**, the module-path version Go tooling consumes |

Each job runs only for its own tag prefix, and **fails if the tag version does not
match the package manifest version** (`package.json` / `pyproject.toml` /
`Cargo.toml`). So bump the manifest first, merge, then tag.

## Registry auth (repo secrets)

The publish jobs read these GitHub Actions repo secrets — a publish only succeeds
once they exist:

| Secret | Used by | How to mint |
|--------|---------|-------------|
| `NPM_TOKEN` | npm job | npm → Access Tokens → Automation token |
| `PYPI_TOKEN` | PyPI job | PyPI → Account → API tokens (username is `__token__`) |
| `CRATES_TOKEN` | crates.io job | crates.io → Account Settings → API Tokens |

Set them with `gh secret set NPM_TOKEN -R david-engelmann/maidan` (reads the value
from stdin — never paste it on the command line). Go needs no secret (tag-only).

## Cutting a release

1. Bump the version in the package manifest (all four track the same client
   version today: `0.1.0`) and merge to `main`.
2. Push the language tag(s), e.g. `git tag sdk-ts-v0.1.0 && git push origin sdk-ts-v0.1.0`.
3. Watch the `sdk-release` run. npm/PyPI/crates.io reject re-publishing an existing
   version, so a repeated tag is a safe no-op there; for Go, delete + re-push the
   `sdk/go/vX.Y.Z` tag only if you must move it.

Dry-run locally before tagging: `npm publish --dry-run` (sdk/typescript),
`python -m build && twine check dist/*` (sdk/python), `cargo publish --dry-run`
(sdk/rust), `go vet ./... && go build ./...` (sdk/go).
