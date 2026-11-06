#!/usr/bin/env python3
"""Tests for oauth-p6-record.py. Run: python3 scripts/test-oauth-p6-record.py

The checks run against the documents in scripts/fixtures/oauth-p6, which copy
the shape Maidan serves (crates/maidan-server/src/oauth/metadata.rs) and the
first P6 attempt's failure (2026-10-28: no client metadata documents, so the
client fell back to dynamic registration)."""

import contextlib
import importlib.util
import io
import json
import sys
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "oauth_p6_record", Path(__file__).with_name("oauth-p6-record.py")
)
p6 = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = p6
_SPEC.loader.exec_module(p6)

FIXTURES = Path(__file__).with_name("fixtures") / "oauth-p6"
ORIGIN = "https://maidan.example.com"


def fixture(name):
    return json.loads((FIXTURES / name).read_text())


def quiet_report():
    return p6.Report()


class Quiet(unittest.TestCase):
    def setUp(self):
        self._out = contextlib.redirect_stdout(io.StringIO())
        self._out.__enter__()

    def tearDown(self):
        self._out.__exit__(None, None, None)


class Discovery(Quiet):
    def test_the_challenge_names_the_resource_document(self):
        r = quiet_report()
        url = p6.check_challenge(
            r, ORIGIN, 401,
            f'Bearer resource_metadata="{ORIGIN}/.well-known/oauth-protected-resource/mcp/streamable"',
        )
        self.assertTrue(r.ok)
        self.assertEqual(url, f"{ORIGIN}/.well-known/oauth-protected-resource/mcp/streamable")

    def test_a_200_or_a_bare_challenge_fails(self):
        r = quiet_report()
        p6.check_challenge(r, ORIGIN, 200, "Bearer")
        self.assertFalse(r.ok)
        self.assertEqual(sum(not c.ok for c in r.checks), 2)

    def test_the_documents_maidan_serves_pass(self):
        r = quiet_report()
        issuer = p6.check_resource_metadata(r, ORIGIN, fixture("resource-metadata.json"))
        self.assertEqual(issuer, ORIGIN)
        self.assertTrue(p6.check_server_metadata(r, issuer, fixture("server-metadata.json")))
        self.assertTrue(r.ok)

    def test_a_resource_document_for_another_resource_fails(self):
        r = quiet_report()
        doc = fixture("resource-metadata.json") | {"resource": f"{ORIGIN}/mcp"}
        p6.check_resource_metadata(r, ORIGIN, doc)
        self.assertFalse(r.ok)

    def test_the_first_p6_attempts_metadata_fails_on_each_gap(self):
        r = quiet_report()
        self.assertFalse(p6.check_server_metadata(r, ORIGIN, fixture("server-metadata-no-cimd.json")))
        failed = {c.name for c in r.checks if not c.ok}
        self.assertEqual(
            failed,
            {
                "PKCE S256 only",
                "client metadata documents advertised",
            },
        )

    def test_a_registration_endpoint_on_another_origin_fails(self):
        r = quiet_report()
        doc = fixture("server-metadata.json") | {"registration_endpoint": "https://evil.example/oauth/register"}
        self.assertFalse(p6.check_server_metadata(r, ORIGIN, doc))
        failed = {c.name for c in r.checks if not c.ok}
        self.assertEqual(failed, {"registration endpoint, if any, is the issuer's"})

    def test_the_server_metadata_url_puts_well_known_before_the_path(self):
        self.assertEqual(p6.as_metadata_url(ORIGIN), f"{ORIGIN}/.well-known/oauth-authorization-server")
        self.assertEqual(
            p6.as_metadata_url(f"{ORIGIN}/tenant/"),
            f"{ORIGIN}/.well-known/oauth-authorization-server/tenant",
        )


class Consent(Quiet):
    def test_a_redirect_with_state_iss_and_code_passes(self):
        r = quiet_report()
        code = p6.check_callback(r, {"code": "c", "state": "s", "iss": ORIGIN}, "s", ORIGIN)
        self.assertEqual(code, "c")

    def test_a_wrong_state_or_missing_iss_fails(self):
        for query in ({"code": "c", "state": "x", "iss": ORIGIN}, {"code": "c", "state": "s"}):
            r = quiet_report()
            self.assertIsNone(p6.check_callback(r, query, "s", ORIGIN))

    def test_an_error_redirect_fails(self):
        r = quiet_report()
        self.assertIsNone(p6.check_callback(r, {"error": "access_denied", "state": "s"}, "s", ORIGIN))
        self.assertFalse(r.ok)


class Token(Quiet):
    def test_the_token_response_passes(self):
        r = quiet_report()
        self.assertIsNotNone(p6.check_token_response(r, 200, "no-store", fixture("token-response.json")))

    def test_a_cacheable_response_or_no_refresh_token_fails(self):
        r = quiet_report()
        self.assertIsNone(p6.check_token_response(r, 200, "max-age=60", fixture("token-response.json")))
        r = quiet_report()
        body = fixture("token-response.json")
        del body["refresh_token"]
        self.assertIsNone(p6.check_token_response(r, 200, "no-store", body))

    def test_a_fingerprint_is_not_the_token(self):
        fp = p6.fingerprint("mdn_fixture_access")
        self.assertEqual(len(fp), 12)
        self.assertNotIn("mdn", fp)


class ToolsList(Quiet):
    def test_an_sse_tools_list_is_read(self):
        message = p6.jsonrpc_result("text/event-stream", (FIXTURES / "tools-list.sse").read_text())
        r = quiet_report()
        self.assertEqual(p6.check_tools_list(r, message), ["claim_next_thread", "post_message"])

    def test_an_empty_list_or_an_error_fails(self):
        for message in ({"jsonrpc": "2.0", "id": 2, "result": {"tools": []}}, {"jsonrpc": "2.0", "id": 2, "error": {"code": -32601}}, None):
            r = quiet_report()
            self.assertEqual(p6.check_tools_list(r, message), [])
            self.assertFalse(r.ok)


class Arguments(Quiet):
    def test_a_non_https_origin_or_a_non_loopback_redirect_is_refused(self):
        for argv in (
            ["--origin", "http://maidan.example.com", "--client-id", "x"],
            ["--origin", ORIGIN, "--client-id", "x", "--redirect-uri", "https://evil.example/cb"],
        ):
            with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                p6.main(argv)




    def test_userinfo_origin_is_refused(self):
        with self.assertRaises(SystemExit):
            p6.main([
                "--origin", "https://user:pass@maidan.world",
                "--client-id", "https://client.example/oauth/client.json",
                "--no-browser",
            ])

    def test_origin_with_path_is_refused(self):
        with self.assertRaises(SystemExit):
            p6.main([
                "--origin", "https://maidan.world/mcp",
                "--client-id", "https://client.example/oauth/client.json",
                "--no-browser",
            ])

    def test_bad_commit_and_out_path_are_refused(self):
        with self.assertRaises(SystemExit):
            p6.main([
                "--origin", "https://maidan.world",
                "--client-id", "https://client.example/oauth/client.json",
                "--commit", "NOTHEX",
                "--no-browser",
            ])
        with self.assertRaises(SystemExit):
            p6.main([
                "--origin", "https://maidan.world",
                "--client-id", "https://client.example/oauth/client.json",
                "--out", "../escape.json",
                "--no-browser",
            ])

    def test_http_client_id_and_non_loopback_redirect_are_refused(self):
        with self.assertRaises(SystemExit):
            p6.main([
                "--origin", "https://maidan.world",
                "--client-id", "http://client.example/oauth/client.json",
                "--no-browser",
            ])
        with self.assertRaises(SystemExit):
            p6.main([
                "--origin", "https://maidan.world",
                "--client-id", "https://client.example/oauth/client.json",
                "--redirect-uri", "http://example.com:33418/callback",
                "--no-browser",
            ])

    def test_finish_strips_raw_tokens_from_the_record(self):
        import tempfile, json
        from pathlib import Path as P
        report = p6.Report()
        report.add("x", "y", True)
        with tempfile.TemporaryDirectory() as tmp:
            out = P(tmp) / "rec.json"
            record = {
                "origin": "https://maidan.world",
                "access_token": "secret-access",
                "refresh_token": "secret-refresh",
                "access_token_sha256": "abc",
                "code": "authcode",
            }
            rc = p6.finish(report, record, str(out))
            self.assertEqual(rc, 0)
            data = json.loads(out.read_text())
            self.assertNotIn("access_token", data)
            self.assertNotIn("refresh_token", data)
            self.assertNotIn("code", data)
            self.assertEqual(data["access_token_sha256"], "abc")


class ValidateArgs(unittest.TestCase):
    def _ns(self, **kwargs):
        import argparse
        defaults = dict(
            origin="https://maidan.world",
            client_id="https://client.example/oauth/client.json",
            redirect_uri="http://127.0.0.1:33418/callback",
            scope=None,
            commit=None,
            out=None,
            no_browser=True,
        )
        defaults.update(kwargs)
        return argparse.Namespace(**defaults)

    def test_accepts_loopback_ipv6_redirect_and_hex_commit(self):
        p6.validate_args(self._ns(
            redirect_uri="http://[::1]:33418/callback",
            commit="d89fee0e121bca5c802f3245b7bba5814e4b4e04",
            out="oauth-p6-record-test.json",
            scope="mcp",
        ))

    def test_rejects_scope_with_newline(self):
        with self.assertRaises(SystemExit):
            p6.validate_args(self._ns(scope="mcp" + chr(10) + "admin"))

if __name__ == "__main__":
    unittest.main(verbosity=1)
