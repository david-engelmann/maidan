#!/usr/bin/env python3
"""Record the OAuth P6 validation run against a public Maidan origin.

Open Work Next 23, docs/OAuth.md "P6 validation record". The script walks the
flow an MCP client walks, in order, and stops at the first step that fails:

  1. discovery   an unauthenticated POST /mcp/streamable answers 401 with a
                 resource_metadata challenge; the RFC 9728 and RFC 8414
                 documents name each other and advertise S256 and client
                 metadata documents, and a registration endpoint, if
                 any, on the issuer itself
  2. consent     the maintainer signs in and clicks Allow in a browser; the
                 redirect back to the loopback callback carries the state
                 sent and `iss` (RFC 9207)
  3. token       the code and PKCE verifier buy an access and refresh token
                 bound to the MCP resource, answered with no-store
  4. tools/list  that token lists tools on /mcp/streamable, and the same token
                 is refused on /mcp (resource binding)

Then it revokes the refresh token, so nothing it minted outlives the run, and
writes a JSON record naming the origin, the version /health reports, the
commit, every check and the tools listed. Tokens and codes are never printed or
recorded; the record holds only their SHA-256 prefix.

Usage:
  python3 scripts/oauth-p6-record.py --origin https://maidan.world \\
      --client-id https://client.example/oauth/client.json --commit <sha>

  --client-id   a client metadata document URL on a host the instance lists in
                MAIDAN_OAUTH_CLIENT_METADATA_HOSTS, or a pre-registered id. Its
                redirect URIs must include the --redirect-uri.
  --redirect-uri  the loopback callback (http://127.0.0.1:33418/callback)
  --scope       the scope to ask for (default: the client's default)
  --commit      the commit the origin runs. Without it the script resolves the
                tag v<version> from /health with `git ls-remote`.
  --out         where the record goes (oauth-p6-record-<UTC date>.json)
  --no-browser  print the authorize URL instead of opening a browser

Exit status: 0 when every check passed, 1 otherwise. Stdlib only.
"""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import http.server
import json
import re
import secrets
import subprocess
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
import webbrowser
from dataclasses import dataclass, field

PROTOCOL_VERSION = "2025-06-18"
REPO = "https://github.com/david-engelmann/maidan"
CONSENT_TIMEOUT_SECS = 600


@dataclass
class Check:
    step: str
    name: str
    ok: bool
    detail: str = ""

    def line(self) -> str:
        return f"{'PASS' if self.ok else 'FAIL'}  {self.step}: {self.name}" + (
            f" ({self.detail})" if self.detail else ""
        )


@dataclass
class Report:
    checks: list[Check] = field(default_factory=list)

    def add(self, step: str, name: str, ok: bool, detail: str = "") -> bool:
        check = Check(step, name, bool(ok), detail)
        self.checks.append(check)
        print(check.line(), flush=True)
        return check.ok

    @property
    def ok(self) -> bool:
        return all(c.ok for c in self.checks)


def fingerprint(secret: str) -> str:
    """A token's SHA-256 prefix: enough to match it to an audit row, useless to replay."""
    return hashlib.sha256(secret.encode()).hexdigest()[:12]


def pkce_pair() -> tuple[str, str]:
    verifier = secrets.token_urlsafe(48)
    challenge = (
        base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest())
        .rstrip(b"=")
        .decode()
    )
    return verifier, challenge


def challenge_metadata_url(www_authenticate: str) -> str | None:
    """The resource_metadata URL in a `Bearer` challenge, if any."""
    m = re.search(r'resource_metadata="([^"]+)"', www_authenticate or "")
    return m.group(1) if m else None


def as_metadata_url(issuer: str) -> str:
    """RFC 8414 section 3.1: the well-known segment goes before the issuer's path."""
    u = urllib.parse.urlsplit(issuer)
    path = u.path.rstrip("/")
    return urllib.parse.urlunsplit(
        (u.scheme, u.netloc, "/.well-known/oauth-authorization-server" + path, "", "")
    )


# Pure checks, run against live documents and against the fixtures in
# scripts/fixtures/oauth-p6 by scripts/test-oauth-p6-record.py.


def check_challenge(report: Report, origin: str, status: int, www_authenticate: str) -> str | None:
    expected = f"{origin}/.well-known/oauth-protected-resource/mcp/streamable"
    report.add("discovery", "unauthenticated POST /mcp/streamable answers 401", status == 401, f"got {status}")
    url = challenge_metadata_url(www_authenticate)
    report.add(
        "discovery",
        "the challenge names the protected resource metadata",
        url == expected,
        f"resource_metadata={url!r}",
    )
    return url


def check_resource_metadata(report: Report, origin: str, prm: object) -> str | None:
    resource = f"{origin}/mcp/streamable"
    if not report.add("discovery", "resource metadata is a JSON object", isinstance(prm, dict)):
        return None
    report.add(
        "discovery",
        "resource metadata names the resource asked for",
        prm.get("resource") == resource,
        f"resource={prm.get('resource')!r}",
    )
    servers = prm.get("authorization_servers") or []
    report.add(
        "discovery",
        "resource metadata names one authorization server",
        isinstance(servers, list) and len(servers) >= 1,
        f"authorization_servers={servers!r}",
    )
    return servers[0] if servers else None


def check_server_metadata(report: Report, issuer: str, meta: object) -> bool:
    if not report.add("discovery", "server metadata is a JSON object", isinstance(meta, dict)):
        return False
    report.add("discovery", "server metadata issuer matches", meta.get("issuer") == issuer, f"issuer={meta.get('issuer')!r}")
    for key in ("authorization_endpoint", "token_endpoint", "revocation_endpoint"):
        report.add("discovery", f"server metadata has {key}", isinstance(meta.get(key), str) and meta[key].startswith("https://"))
    report.add(
        "discovery",
        "PKCE S256 only",
        meta.get("code_challenge_methods_supported") == ["S256"],
        f"{meta.get('code_challenge_methods_supported')!r}",
    )
    report.add(
        "discovery",
        "client metadata documents advertised",
        meta.get("client_id_metadata_document_supported") is True,
    )
    # RFC 7591 registration is optional (MAIDAN_OAUTH_DYNAMIC_REGISTRATION),
    # but when advertised it must be this issuer's own endpoint.
    registration = meta.get("registration_endpoint")
    report.add(
        "discovery",
        "registration endpoint, if any, is the issuer's",
        registration is None or registration == f"{issuer}/oauth/register",
        f"registration_endpoint={registration!r}",
    )
    return report.ok


def check_callback(report: Report, query: dict[str, str], state: str, issuer: str) -> str | None:
    if "error" in query:
        report.add("consent", "the server redirected back with a code", False, f"error={query.get('error')!r} {query.get('error_description', '')}")
        return None
    report.add("consent", "the redirect carries the state sent", query.get("state") == state)
    report.add("consent", "the redirect carries iss", query.get("iss") == issuer, f"iss={query.get('iss')!r}")
    code = query.get("code")
    report.add("consent", "the redirect carries a code", bool(code))
    return code if report.ok else None


def check_token_response(report: Report, status: int, cache_control: str, body: object) -> dict | None:
    report.add("token", "the token endpoint answers 200", status == 200, f"got {status}")
    report.add("token", "the token response is no-store", "no-store" in (cache_control or "").lower())
    if not isinstance(body, dict):
        report.add("token", "the token response is a JSON object", False)
        return None
    report.add("token", "token_type is Bearer", str(body.get("token_type", "")).lower() == "bearer")
    report.add("token", "an access token is issued", isinstance(body.get("access_token"), str) and bool(body.get("access_token")))
    report.add("token", "a refresh token is issued", isinstance(body.get("refresh_token"), str) and bool(body.get("refresh_token")))
    report.add("token", "expires_in is set", isinstance(body.get("expires_in"), int) and body["expires_in"] > 0)
    return body if report.ok else None


def jsonrpc_result(content_type: str, text: str) -> object:
    """The JSON-RPC message in a JSON body or in an SSE stream's data lines."""
    if "text/event-stream" in (content_type or ""):
        data = [line[5:].strip() for line in text.splitlines() if line.startswith("data:")]
        text = "\n".join(data)
    try:
        return json.loads(text)
    except ValueError:
        return None


def check_tools_list(report: Report, message: object) -> list[str]:
    tools = message.get("result", {}).get("tools") if isinstance(message, dict) else None
    ok = isinstance(tools, list) and len(tools) > 0
    report.add("tools/list", "tools/list returns tools", ok, f"{len(tools) if isinstance(tools, list) else 0} tools")
    return sorted(t.get("name", "") for t in tools) if ok else []


# Network.


def http(method: str, url: str, *, headers: dict | None = None, data: bytes | None = None):
    req = urllib.request.Request(url, data=data, method=method, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.status, resp.headers, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as err:
        return err.code, err.headers, err.read().decode("utf-8", "replace")


def get_json(url: str) -> object:
    status, _, text = http("GET", url, headers={"Accept": "application/json"})
    try:
        return json.loads(text) if status == 200 else None
    except ValueError:
        return None


def form(fields: dict) -> bytes:
    return urllib.parse.urlencode({k: v for k, v in fields.items() if v is not None}).encode()


def wait_for_callback(redirect_uri: str) -> dict[str, str] | None:
    """Serve the loopback redirect once and return its query."""
    u = urllib.parse.urlsplit(redirect_uri)
    got: dict[str, str] = {}
    done = threading.Event()

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802
            parts = urllib.parse.urlsplit(self.path)
            if parts.path != u.path:
                self.send_response(404)
                self.end_headers()
                return
            got.update(dict(urllib.parse.parse_qsl(parts.query)))
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.end_headers()
            self.wfile.write(b"Recorded. You can close this tab.\n")
            done.set()

        def log_message(self, *args):
            pass

    server = http.server.HTTPServer((u.hostname or "127.0.0.1", u.port or 80), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    finished = done.wait(CONSENT_TIMEOUT_SECS)
    server.shutdown()
    return got if finished else None


def resolve_commit(version: str | None) -> str | None:
    if not version:
        return None
    try:
        out = subprocess.run(
            ["git", "ls-remote", REPO, f"refs/tags/v{version}^{{}}", f"refs/tags/v{version}"],
            capture_output=True, text=True, timeout=30, check=False,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    lines = [line.split()[0] for line in out.splitlines() if line.strip()]
    peeled = [line.split()[0] for line in out.splitlines() if line.endswith("^{}")]
    return (peeled or lines or [None])[0]


def mcp_post(url: str, token: str | None, message: dict, protocol: bool) -> tuple[int, object]:
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if protocol:
        headers["MCP-Protocol-Version"] = PROTOCOL_VERSION
    status, resp_headers, text = http("POST", url, headers=headers, data=json.dumps(message).encode())
    return status, jsonrpc_result(resp_headers.get("Content-Type", "") if resp_headers else "", text)


def run(args: argparse.Namespace) -> int:
    origin = args.origin.rstrip("/")
    report = Report()
    record: dict = {
        "origin": origin,
        "started_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "client_id": args.client_id,
        "redirect_uri": args.redirect_uri,
        "protocol_version": PROTOCOL_VERSION,
    }
    health = get_json(f"{origin}/health")
    record["version"] = health.get("version") if isinstance(health, dict) else None
    record["commit"] = args.commit or resolve_commit(record["version"])
    report.add("record", "the commit the origin runs is known", bool(record["commit"]), f"version={record['version']!r}")

    initialize = {
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": {"name": "oauth-p6-record", "version": "1"}},
    }
    # 1. discovery
    status, headers, _ = http(
        "POST", f"{origin}/mcp/streamable",
        headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
        data=json.dumps(initialize).encode(),
    )
    prm_url = check_challenge(report, origin, status, headers.get("WWW-Authenticate", "") if headers else "")
    issuer = check_resource_metadata(report, origin, get_json(prm_url)) if prm_url else None
    meta = get_json(as_metadata_url(issuer)) if issuer else None
    if not issuer or not check_server_metadata(report, issuer, meta):
        return finish(report, record, args.out)

    # 2. consent
    verifier, challenge = pkce_pair()
    state = secrets.token_urlsafe(16)
    resource = f"{origin}/mcp/streamable"
    url = meta["authorization_endpoint"] + "?" + urllib.parse.urlencode({
        k: v for k, v in {
            "response_type": "code", "client_id": args.client_id, "redirect_uri": args.redirect_uri,
            "code_challenge": challenge, "code_challenge_method": "S256", "state": state,
            "resource": resource, "scope": args.scope,
        }.items() if v
    })
    print(f"\nSign in and click Allow within {CONSENT_TIMEOUT_SECS // 60} minutes:\n  {url}\n", flush=True)
    if not args.no_browser:
        webbrowser.open(url)
    query = wait_for_callback(args.redirect_uri)
    if query is None:
        report.add("consent", "the browser came back to the callback", False, "timed out")
        return finish(report, record, args.out)
    record["consented_at"] = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
    code = check_callback(report, query, state, issuer)
    if not code:
        return finish(report, record, args.out)

    # 3. token
    status, headers, text = http(
        "POST", meta["token_endpoint"],
        headers={"Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"},
        data=form({
            "grant_type": "authorization_code", "code": code, "code_verifier": verifier,
            "redirect_uri": args.redirect_uri, "client_id": args.client_id, "resource": resource,
        }),
    )
    try:
        body = json.loads(text)
    except ValueError:
        body = None
    tokens = check_token_response(report, status, headers.get("Cache-Control", "") if headers else "", body)
    if not tokens:
        return finish(report, record, args.out)
    access, refresh = tokens["access_token"], tokens["refresh_token"]
    record["scope"] = tokens.get("scope")
    record["access_token_sha256"] = fingerprint(access)
    record["refresh_token_sha256"] = fingerprint(refresh)

    # 4. tools/list
    status, init = mcp_post(resource, access, initialize, protocol=False)
    report.add("tools/list", "initialize with the token answers", status == 200 and isinstance(init, dict) and "result" in init, f"got {status}")
    mcp_post(resource, access, {"jsonrpc": "2.0", "method": "notifications/initialized"}, protocol=True)
    _, listed = mcp_post(resource, access, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"}, protocol=True)
    record["tools"] = check_tools_list(report, listed)
    status, _ = mcp_post(f"{origin}/mcp", access, initialize, protocol=False)
    report.add("tools/list", "the token is refused on /mcp (bound to /mcp/streamable)", status == 401, f"got {status}")

    # Leave nothing behind: revoking the refresh token revokes the grant's tokens.
    status, _, _ = http(
        "POST", meta["revocation_endpoint"],
        headers={"Content-Type": "application/x-www-form-urlencoded"},
        data=form({"token": refresh, "token_type_hint": "refresh_token", "client_id": args.client_id}),
    )
    report.add("revoke", "the refresh token is revoked", status == 200, f"got {status}")
    return finish(report, record, args.out)


def finish(report: Report, record: dict, out: str | None) -> int:
    record["finished_at"] = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
    record["passed"] = report.ok
    record["checks"] = [c.__dict__ for c in report.checks]
    # Defense in depth: never serialize raw secrets if a caller stuffed them in.
    banned = {"access_token", "refresh_token", "id_token", "code", "password", "client_secret"}
    for key in list(record):
        if key in banned or (key.endswith("_token") and not key.endswith("_sha256")):
            record.pop(key, None)
    path = out or f"oauth-p6-record-{dt.datetime.now(dt.timezone.utc):%Y-%m-%d}.json"
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(record, fh, indent=2)
        fh.write("\n")
    print(f"\n{'PASSED' if report.ok else 'FAILED'}: record written to {path}")
    return 0 if report.ok else 1



def validate_args(args: argparse.Namespace) -> None:
    """Fail closed on unsafe recorder inputs before any network I/O."""
    origin = urllib.parse.urlsplit(args.origin)
    if origin.scheme != "https":
        raise SystemExit("oauth-p6-record: --origin must be https (public instance only)")
    if origin.username or origin.password or "@" in (origin.netloc or ""):
        raise SystemExit("oauth-p6-record: --origin must not carry userinfo")
    if not origin.hostname or "." not in origin.hostname:
        raise SystemExit("oauth-p6-record: --origin hostname looks incomplete")
    if origin.query or origin.fragment:
        raise SystemExit("oauth-p6-record: --origin must not carry a query or fragment")
    if origin.path not in ("", "/"):
        raise SystemExit("oauth-p6-record: --origin must be an origin (no path)")

    client = args.client_id.strip()
    if not client or len(client) > 512:
        raise SystemExit("oauth-p6-record: --client-id must be 1..512 characters")
    if client.startswith("http://"):
        raise SystemExit("oauth-p6-record: --client-id URL must be https (or a pre-registered id)")
    if client.startswith("https://"):
        cu = urllib.parse.urlsplit(client)
        if cu.username or cu.password or not cu.hostname:
            raise SystemExit("oauth-p6-record: --client-id URL is malformed")
    elif not re.fullmatch(r"[A-Za-z0-9._~-]{8,128}", client):
        raise SystemExit("oauth-p6-record: --client-id must be an https URL or a pre-registered id")

    redirect = urllib.parse.urlsplit(args.redirect_uri)
    host_ok = redirect.hostname in ("127.0.0.1", "localhost", "::1")
    if redirect.scheme != "http" or not host_ok or not redirect.port:
        raise SystemExit("oauth-p6-record: --redirect-uri must be loopback http with a port")
    if (not redirect.path.startswith("/")) or (".." in redirect.path.split("/")):
        raise SystemExit("oauth-p6-record: --redirect-uri path must be absolute without ..")
    if redirect.query or redirect.fragment:
        raise SystemExit("oauth-p6-record: --redirect-uri must not carry a query or fragment")

    if args.commit is not None:
        if not re.fullmatch(r"[0-9a-f]{7,40}", args.commit):
            raise SystemExit("oauth-p6-record: --commit must be a 7..40 hex SHA")

    if args.scope is not None:
        if (not args.scope) or len(args.scope) > 256 or ("\n" in args.scope) or ("\r" in args.scope):
            raise SystemExit("oauth-p6-record: --scope must be 1..256 printable characters")

    if args.out is not None:
        import os
        out = args.out
        if out != os.path.basename(out) or out in (".", "..") or "/" in out or "\\" in out:
            raise SystemExit("oauth-p6-record: --out must be a basename in the current directory")
        if not re.fullmatch(r"[A-Za-z0-9._-]{1,128}", out):
            raise SystemExit("oauth-p6-record: --out basename has illegal characters")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--origin", required=True)
    parser.add_argument("--client-id", required=True)
    parser.add_argument("--redirect-uri", default="http://127.0.0.1:33418/callback")
    parser.add_argument("--scope")
    parser.add_argument("--commit")
    parser.add_argument("--out")
    parser.add_argument("--no-browser", action="store_true")
    args = parser.parse_args(argv)
    try:
        validate_args(args)
    except SystemExit as exc:
        msg = str(exc)
        if msg.startswith("oauth-p6-record: "):
            msg = msg[len("oauth-p6-record: "):]
        parser.error(msg)
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
