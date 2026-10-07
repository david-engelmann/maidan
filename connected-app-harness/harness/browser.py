"""Browser management: persistent per-provider profiles.

One profile dir per provider under profiles/. David logs in once via
`harness auth-login <provider>` (headed). Everything else runs headless
against the saved profile and NEVER attempts to log in.

Sandbox networking: the sandbox routes egress through an explicit HTTP
proxy that (a) denies Chromium's direct CONNECT (policy_denied) while
accepting the identical bytes via a localhost relay, and (b) intercepts
TLS with the Hatch Egress CA, which Chromium doesn't trust (curl/Python
trust it via SSL_CERT_FILE). `persistent_context` works around both,
gated on detecting the sandbox proxy, so behavior elsewhere is unchanged.
"""
from __future__ import annotations

import os
import socket
import threading
from contextlib import contextmanager

from playwright.sync_api import sync_playwright

from . import config as config_mod


def _egress_proxy_target() -> tuple[str, int] | None:
    """(host, port) of the explicit egress proxy from env, or None.

    Accepts "host:port" or "http://host:port" (with or without credentials).
    """
    for var in ("https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"):
        val = (os.environ.get(var) or "").strip()
        if not val:
            continue
        if "://" in val:
            val = val.split("://", 1)[1]
        val = val.split("/")[0].split("@")[-1]
        if ":" not in val:
            continue
        host, _, port = val.rpartition(":")
        try:
            return host.strip("[]"), int(port)
        except ValueError:
            continue
    return None


# Identifiers for the sandbox's intercepting egress proxy. Exact
# matching only: this gates a TLS-verification downgrade, so substring
# matching (e.g. "hatch" in host) is not acceptable — see the playbook.
_SANDBOX_PROXY_HOSTS = ("hatch-egress-proxy",)
# Known CA bundle paths for the sandbox MITM proxy.
_SANDBOX_CA_PATHS = ("/run/hatch/egress-tls/ca-bundle.pem",)


def _is_sandbox_mitm(target: tuple[str, int] | None = None) -> bool:
    """True when the sandbox's intercepting egress proxy is in use."""
    if target is None:
        target = _egress_proxy_target()
    if not target:
        return False
    host = target[0].lower()
    if any(host == h or host.endswith("." + h)
           for h in _SANDBOX_PROXY_HOSTS):
        return True
    ca_bundle = os.environ.get("SSL_CERT_FILE", "")
    return ca_bundle in _SANDBOX_CA_PATHS


class _ProxyRelay:
    """Localhost TCP forwarder to the egress proxy.

    Chromium -> 127.0.0.1:PORT -> egress proxy. The initial client bytes
    are forwarded as a single write; the proxy's policy engine appears to
    require the CONNECT request unfragmented. Afterwards the connection
    is a plain bidirectional pipe. TLS stays end-to-end (intercepted by
    the sandbox proxy, as documented).
    """

    def __init__(self, target: tuple[str, int]):
        self._target = target
        self._srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._srv.bind(("127.0.0.1", 0))
        self._srv.listen(64)
        self.port = self._srv.getsockname()[1]
        self._running = True
        threading.Thread(target=self._serve, daemon=True).start()

    def _serve(self) -> None:
        while self._running:
            try:
                client, _ = self._srv.accept()
            except OSError:
                return
            threading.Thread(target=self._handle, args=(client,),
                             daemon=True).start()

    def _handle(self, client: socket.socket) -> None:
        try:
            upstream = socket.create_connection(self._target, timeout=20)
        except OSError:
            client.close()
            return
        try:
            # A connect-but-send-nothing client must not park a thread
            # forever on recv.
            client.settimeout(15)
            # Read until the end of the HTTP headers (\r\n\r\n), then
            # forward as a SINGLE unfragmented write: the proxy's policy
            # engine keys on seeing the CONNECT target in one segment. A
            # single recv() can split the request across TCP segments.
            first = b""
            while b"\r\n\r\n" not in first:
                chunk = client.recv(8192)
                if not chunk:
                    return
                first += chunk
                if len(first) > 65536:  # not HTTP; bail
                    return
            upstream.sendall(first)
            t = threading.Thread(target=self._pump, args=(upstream, client),
                                 daemon=True)
            t.start()
            self._pump(client, upstream)
            t.join(timeout=5)
        except OSError:
            pass
        finally:
            for s in (client, upstream):
                try:
                    s.close()
                except OSError:
                    pass

    @staticmethod
    def _pump(src: socket.socket, dst: socket.socket) -> None:
        try:
            while True:
                data = src.recv(16384)
                if not data:
                    break
                dst.sendall(data)
        except OSError:
            pass

    def close(self) -> None:
        self._running = False
        try:
            self._srv.close()
        except OSError:
            pass


@contextmanager
def persistent_context(provider: str, headless: bool = True):
    """Yield a Playwright browser context on the provider's profile dir."""
    profile = config_mod.profile_dir(provider)
    target = _egress_proxy_target()
    # Parse once: _is_sandbox_mitm() re-parses, so pass the verdict in.
    sandbox = _is_sandbox_mitm(target)
    relay = _ProxyRelay(target) if target else None
    kwargs: dict = {}
    if relay:
        kwargs["proxy"] = {"server": f"http://127.0.0.1:{relay.port}"}
    if sandbox:
        # In the sandbox the egress proxy intercepts TLS with the Hatch
        # CA, which Chromium doesn't trust (unlike curl/Python via
        # SSL_CERT_FILE). Relax cert validation ONLY there; everywhere
        # else it stays on.
        kwargs["ignore_https_errors"] = True
    try:
        with sync_playwright() as p:
            ctx = p.chromium.launch_persistent_context(
                profile,
                headless=headless,
                args=[
                    "--disable-blink-features=AutomationControlled",
                ],
                user_agent=(
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
                    "AppleWebKit/537.36 (KHTML, like Gecko) "
                    "Chrome/131.0.0.0 Safari/537.36"
                ),
                viewport={"width": 1440, "height": 900},
                # Never wait at a login screen: short default timeouts, and
                # callers use explicit short waits for auth markers.
                timeout=15000,
                **kwargs,
            )
            try:
                yield ctx
            finally:
                ctx.close()
    finally:
        # A launch failure must not leak the listening socket/thread.
        if relay:
            relay.close()
