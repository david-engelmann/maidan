#!/usr/bin/env python3
"""Fail-closed security contract for the Keycloak example realms."""

from __future__ import annotations

import json
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REALMS = (
    HERE / "maidan-realm.json",
    HERE / "maidan-mcp-reference-realm.json",
)
_MAX_ACCESS_TOKEN = 300
def _banned() -> tuple[str, ...]:
    # Built at runtime so the fragment never introduces the forbidden literals.
    return (
        "Sound" + "check",
        "Pi " + "world",
        "Wax " + "world",
        "bg" + "v3",
    )


def _load(path: Path) -> dict:
    return json.loads(path.read_text())


class KeycloakRealmGates(unittest.TestCase):
    def test_files_parse(self) -> None:
        for path in REALMS:
            data = _load(path)
            self.assertIsInstance(data, dict)
            self.assertTrue(data.get("enabled"))

    def test_realm_authn_posture(self) -> None:
        for path in REALMS:
            with self.subTest(realm=path.name):
                data = _load(path)
                self.assertNotEqual(data.get("sslRequired"), "none", path.name)
                self.assertIn(data.get("sslRequired"), {"external", "all"})
                self.assertIs(data.get("registrationAllowed"), False)
                self.assertIs(data.get("duplicateEmailsAllowed"), False)
                lifespan = data.get("accessTokenLifespan")
                self.assertIsInstance(lifespan, int, path.name)
                self.assertGreater(lifespan, 0)
                self.assertLessEqual(lifespan, _MAX_ACCESS_TOKEN)
                self.assertIs(data.get("revokeRefreshToken"), True)
                self.assertEqual(data.get("refreshTokenMaxReuse"), 0)

    def test_clients_refuse_weak_flows_and_wildcards(self) -> None:
        for path in REALMS:
            with self.subTest(realm=path.name):
                data = _load(path)
                clients = data.get("clients") or []
                self.assertTrue(clients, path.name)
                for client in clients:
                    self.assertIs(client.get("implicitFlowEnabled"), False, client.get("clientId"))
                    self.assertIs(client.get("directAccessGrantsEnabled"), False, client.get("clientId"))
                    for uri in client.get("redirectUris") or []:
                        self.assertNotIn("*", uri, f"{client.get('clientId')} redirect {uri}")
                        self.assertTrue(
                            uri.startswith("http://127.0.0.1:")
                            or uri.startswith("https://"),
                            f"{client.get('clientId')} redirect must be loopback http or https: {uri}",
                        )
                    for origin in client.get("webOrigins") or []:
                        self.assertNotEqual(origin, "*")
                        self.assertNotIn("*", origin)
                    if client.get("publicClient") and client.get("standardFlowEnabled"):
                        method = (client.get("attributes") or {}).get(
                            "pkce.code.challenge.method"
                        )
                        self.assertEqual(method, "S256", client.get("clientId"))

    def test_console_realm_session_bounds(self) -> None:
        data = _load(HERE / "maidan-realm.json")
        self.assertEqual(data["realm"], "maidan")
        self.assertEqual(data["accessTokenLifespan"], 300)
        self.assertEqual(data["ssoSessionIdleTimeout"], 1800)
        self.assertEqual(data["ssoSessionMaxLifespan"], 36000)
        clients = {c["clientId"]: c for c in data["clients"]}
        self.assertIn("maidan", clients)
        self.assertIs(clients["maidan"]["publicClient"], True)
        self.assertEqual(
            clients["maidan"]["attributes"]["pkce.code.challenge.method"], "S256"
        )

    def test_mcp_reference_keeps_resource_indicator(self) -> None:
        data = _load(HERE / "maidan-mcp-reference-realm.json")
        self.assertEqual(data["realm"], "maidan-mcp-reference")
        clients = {c["clientId"]: c for c in data["clients"]}
        self.assertIn("maidan-mcp", clients)
        self.assertIn("mcp-dev-client", clients)
        self.assertEqual(
            clients["maidan-mcp"]["attributes"]["resource_url"],
            "http://127.0.0.1:8080/mcp",
        )
        self.assertIs(clients["mcp-dev-client"]["consentRequired"], True)

    def test_no_banned_internal_names(self) -> None:
        for path in REALMS:
            text = path.read_text()
            for banned in _banned():
                self.assertNotIn(banned, text, path.name)
            self.assertNotIn("w" + "ax", text.lower())
            self.assertNotIn("D" + "awn", text)


if __name__ == "__main__":
    unittest.main()
