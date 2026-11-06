#!/usr/bin/env bash
# Contract gate for the production server image, the operator CLI image, the
# try.maidan.world demo image, and the deploy workflows that pin them.
#
# Usage: scripts/check-server-image-gates.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import re
import sys
from pathlib import Path

errors = []
checked = 0

def require(cond, msg):
    if not cond:
        errors.append(msg)

server = Path("crates/maidan-server/Dockerfile").read_text()
checked += 1
require(re.search(r"(?m)^USER\s+nonroot:nonroot\s*$", server),
        "crates/maidan-server/Dockerfile must USER nonroot:nonroot")
require("cgr.dev/chainguard/glibc-dynamic@sha256:" in server,
        "crates/maidan-server/Dockerfile runtime must pin chainguard by digest")
require(re.search(r"COPY --from=builder --chmod=0555\s+.*/maidan-server\s+/usr/local/bin/maidan-server", server),
        "crates/maidan-server/Dockerfile must COPY the binary with --chmod=0555")
require(re.search(r"(?m)^STOPSIGNAL\s+SIGTERM\s*$", server),
        "crates/maidan-server/Dockerfile must STOPSIGNAL SIGTERM")
require('ENTRYPOINT ["/usr/local/bin/maidan-server"]' in server,
        "crates/maidan-server/Dockerfile must use an absolute ENTRYPOINT")

cli = Path("docker/Dockerfile.cli").read_text()
checked += 1
require(re.search(r"(?m)^USER\s+nonroot:nonroot\s*$", cli),
        "docker/Dockerfile.cli must USER nonroot:nonroot")
require("cgr.dev/chainguard/glibc-dynamic@sha256:" in cli,
        "docker/Dockerfile.cli must pin chainguard by digest")
require("--chmod=0555" in cli and "maidan" in cli,
        "docker/Dockerfile.cli must COPY the CLI with --chmod=0555")
require('org.opencontainers.image.licenses="MIT"' in cli,
        "docker/Dockerfile.cli must declare MIT license label")
require(re.search(r"(?m)^STOPSIGNAL\s+SIGTERM\s*$", cli),
        "docker/Dockerfile.cli must STOPSIGNAL SIGTERM")

demo = Path("deploy/cloudflare/demo/Dockerfile").read_text()
checked += 1
require(re.search(r"(?m)^USER\s+demo\s*$", demo),
        "deploy/cloudflare/demo/Dockerfile must USER demo")
require(re.search(r"(?m)^ARG\s+SERVER_IMAGE\s*$", demo),
        "deploy/cloudflare/demo/Dockerfile must require SERVER_IMAGE with no default tag")
require("COPY --from=server --chmod=0555" in demo,
        "deploy/cloudflare/demo/Dockerfile must COPY maidan-server with --chmod=0555")
require("COPY --chmod=0555 deploy/cloudflare/demo/entrypoint.sh" in demo,
        "deploy/cloudflare/demo/Dockerfile must COPY entrypoint with --chmod=0555")
require(re.search(r"(?m)^STOPSIGNAL\s+SIGTERM\s*$", demo),
        "deploy/cloudflare/demo/Dockerfile must STOPSIGNAL SIGTERM")
require("digest" in demo.lower() and "sha256" in demo,
        "deploy/cloudflare/demo/Dockerfile must document digest-pinned SERVER_IMAGE")

demo_wf = Path(".github/workflows/deploy-demo.yml").read_text()
checked += 1
require('TAG is empty' in demo_wf or '[ -n "${TAG:-}" ]' in demo_wf,
        "deploy-demo.yml must refuse an empty TAG")
require(re.search(r"digest=.*amd64|amd64.*digest", demo_wf) or "digest=\"$(amd64" in demo_wf,
        "deploy-demo.yml must capture the amd64 digest before build")
require("sha256:[0-9a-f]{64}" in demo_wf or r"sha256:[0-9a-f]{64}" in demo_wf,
        "deploy-demo.yml must validate the amd64 digest shape")
require("cosign verify" in demo_wf,
        "deploy-demo.yml must cosign-verify the release image")

prod_wf = Path(".github/workflows/deploy-prod.yml").read_text()
checked += 1
require('TAG is empty' in prod_wf or '[ -n "${TAG:-}" ]' in prod_wf,
        "deploy-prod.yml must refuse an empty TAG")
require(re.search(r"\^v\[0-9\]", prod_wf) or "^v[0-9]+\\.[0-9]+\\.[0-9]+$" in prod_wf
        or r"^v[0-9]+\.[0-9]+\.[0-9]+$" in prod_wf,
        "deploy-prod.yml must require a vN.N.N release tag")

clean = Path(".github/workflows/clean-install.yml").read_text()
checked += 1
require("STRICT=1" in clean,
        "clean-install.yml must run the installer with STRICT=1")
require("not a release tag" in clean,
        "clean-install.yml must refuse a non-vN.N.N tag")

if errors:
    print("server-image gates:", file=sys.stderr)
    for e in errors:
        print(f"  {e}", file=sys.stderr)
    sys.exit(1)
print(f"server-image gates: {checked} check(s) ok")
PY
