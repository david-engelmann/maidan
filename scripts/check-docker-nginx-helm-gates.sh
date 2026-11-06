#!/usr/bin/env bash
# Contract gate for the scale-out nginx proxy, Postgres image hardening, the
# OTLP collector memory bound, and the Helm values schema that refuses a
# non-root-less / unbounded chart render.
#
# Usage: scripts/check-docker-nginx-helm-gates.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import json
import re
import sys
from pathlib import Path

errors = []
checked = 0

def require(cond, msg):
    if not cond:
        errors.append(msg)

# --- nginx-scale.conf ---
nginx = Path("docker/nginx-scale.conf").read_text()
checked += 1
require(re.search(r"(?m)^\s*server_tokens\s+off\s*;", nginx),
        "docker/nginx-scale.conf must set server_tokens off")
require(re.search(r"(?m)^\s*client_max_body_size\s+\d+m\s*;", nginx),
        "docker/nginx-scale.conf must set client_max_body_size")
for header in ("X-Content-Type-Options", "X-Frame-Options", "Referrer-Policy"):
    require(header in nginx,
            f"docker/nginx-scale.conf must add_header {header}")
require("proxy_hide_header Server" in nginx,
        "docker/nginx-scale.conf must proxy_hide_header Server")

# --- Dockerfile.db ---
db = Path("docker/Dockerfile.db").read_text()
checked += 1
require("chmod 700 /archive" in db or "chmod 0700 /archive" in db,
        "docker/Dockerfile.db must chmod 700 /archive")
require("chown postgres:postgres /archive" in db,
        "docker/Dockerfile.db must chown /archive to postgres")
require("docker-entrypoint-initdb.d" in db and "rm -rf" in db,
        "docker/Dockerfile.db must wipe docker-entrypoint-initdb.d")

# --- otel collector ---
otel = Path("docker/otel-collector-config.yaml").read_text()
checked += 1
require("memory_limiter" in otel,
        "docker/otel-collector-config.yaml must define memory_limiter")
require(re.search(r"processors:\s*\n(?:.*\n)*?\s*-\s*memory_limiter", otel)
        or ("processors: [memory_limiter]" in otel)
        or re.search(r"processors:\s*\[memory_limiter\]", otel),
        "docker/otel-collector-config.yaml pipelines must use memory_limiter")
# both pipelines mention the processor
require(otel.count("memory_limiter") >= 3,
        "docker/otel-collector-config.yaml must attach memory_limiter to pipelines")

# --- demo image nonroot ---
demo = Path("deploy/cloudflare/demo/Dockerfile").read_text()
checked += 1
require(re.search(r"(?m)^USER\s+demo\s*$", demo),
        "deploy/cloudflare/demo/Dockerfile must USER demo")

# --- Helm values.schema.json ---
schema_path = Path("helm/maidan/values.schema.json")
require(schema_path.is_file(), "helm/maidan/values.schema.json must exist")
checked += 1
schema = json.loads(schema_path.read_text())
props = schema.get("properties") or {}
psc = (props.get("podSecurityContext") or {}).get("properties") or {}
require(psc.get("runAsNonRoot", {}).get("const") is True,
        "values.schema.json must require podSecurityContext.runAsNonRoot const true")
res = (props.get("resources") or {}).get("properties") or {}
require("memory" in ((res.get("limits") or {}).get("properties") or {}),
        "values.schema.json must require resources.limits.memory")
# defaults must validate
values = Path("helm/maidan/values.yaml")
# lightweight check: required keys present in values text
vt = values.read_text()
require("runAsNonRoot: true" in vt, "helm/maidan/values.yaml must set runAsNonRoot: true")
require(re.search(r"(?m)^resources:\n(?:.*\n)*?  limits:\n(?:.*\n)*?    memory:", vt),
        "helm/maidan/values.yaml must set resources.limits.memory")

if errors:
    print("docker-nginx-helm gates:", file=sys.stderr)
    for e in errors:
        print(f"  {e}", file=sys.stderr)
    sys.exit(1)
print(f"docker-nginx-helm gates: {checked} check(s) ok")
PY
