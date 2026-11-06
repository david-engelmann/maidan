#!/usr/bin/env python3
"""Fail-closed gates for listing catalog pins and listing-prompts inputs."""

from __future__ import annotations

import importlib.util
import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location(
    "listing_prompts", ROOT / "scripts" / "listing-prompts.py"
)
lp = importlib.util.module_from_spec(_spec)
assert _spec.loader is not None
_spec.loader.exec_module(lp)

REGISTRY = ROOT / "listings" / "registry" / "server.json"
DOCKER = ROOT / "listings" / "docker" / "server.yaml"
TOOLHIVE = ROOT / "listings" / "toolhive" / "server.json"
PROMPTS = ROOT / "listings" / "prompts.json"

_IMAGE_PIN = re.compile(
    r"^ghcr\.io/david-engelmann/maidan-server:v\d+\.\d+\.\d+$"
)
_RELEASE_VER = re.compile(r"^v?\d+\.\d+\.\d+$")


class ListingCatalogGates(unittest.TestCase):
    def test_registry_pins_and_loopback_transport(self) -> None:
        data = json.loads(REGISTRY.read_text())
        self.assertEqual(
            data["repository"]["url"], "https://github.com/david-engelmann/maidan"
        )
        self.assertRegex(data["version"], _RELEASE_VER)
        for icon in data["icons"]:
            self.assertTrue(icon["src"].startswith("https://"))
            self.assertIn("david-engelmann/maidan", icon["src"])
        for pkg in data["packages"]:
            self.assertRegex(pkg["version"], _RELEASE_VER)
            self.assertNotIn("latest", pkg["version"])
            url = pkg["transport"]["url"]
            self.assertTrue(
                url.startswith("http://127.0.0.1:")
                or url.startswith("https://maidan.world/"),
                url,
            )
            self.assertEqual(pkg["transport"]["type"], "streamable-http")
            self.assertEqual(pkg["registryType"], "oci")
            self.assertEqual(pkg["registryBaseUrl"], "https://ghcr.io")

    def test_docker_and_toolhive_image_pins(self) -> None:
        docker = DOCKER.read_text()
        m = re.search(r"(?m)^image:\s*(\S+)\s*$", docker)
        self.assertIsNotNone(m)
        self.assertRegex(m.group(1), _IMAGE_PIN)
        self.assertNotIn(":latest", docker)
        th = json.loads(TOOLHIVE.read_text())
        self.assertRegex(th["image"], _IMAGE_PIN)
        self.assertEqual(th["transport"], "streamable-http")
        self.assertEqual(th["repository_url"], "https://github.com/david-engelmann/maidan")
        self.assertEqual(th["status"], "active")

    def test_prompts_shape(self) -> None:
        data = json.loads(PROMPTS.read_text())
        self.assertTrue(data["prompts"])
        self.assertTrue(data["listings"])
        for key, prompt in data["prompts"].items():
            self.assertRegex(key, r"^[a-z][a-z0-9-]{0,63}$")
            self.assertTrue(prompt["prompt"].strip())
            self.assertTrue(prompt["expect"])
            for tool in prompt["expect"]:
                self.assertRegex(tool, r"^[a-z][a-z0-9_]{0,127}$")
        for listing, keys in data["listings"].items():
            self.assertRegex(listing, r"^[a-z][a-z0-9-]{0,63}$")
            self.assertTrue(keys)
            for k in keys:
                self.assertIn(k, data["prompts"])

    def test_listing_prompts_validators(self) -> None:
        data = json.loads(PROMPTS.read_text())
        known = next(iter(data["listings"]))
        self.assertEqual(lp.validate_listing(known, data), known)
        with self.assertRaises(ValueError):
            lp.validate_listing("../x", data)
        with self.assertRaises(ValueError):
            lp.validate_listing("no-such-listing-xyz", data)
        self.assertEqual(lp.validate_timeout(300), 300)
        with self.assertRaises(ValueError):
            lp.validate_timeout(0)
        with self.assertRaises(ValueError):
            lp.validate_timeout(601)
        self.assertEqual(lp.validate_run_cmd("claude -p {prompt}"), "claude -p {prompt}")
        with self.assertRaises(ValueError):
            lp.validate_run_cmd("claude -p {prompt}; rm -rf /")
        with self.assertRaises(ValueError):
            lp.validate_run_cmd("claude " + "$" + "(id)")
        with self.assertRaises(ValueError):
            lp.validate_run_cmd("")

    def test_no_banned_internal_names(self) -> None:
        paths = (
            REGISTRY,
            DOCKER,
            TOOLHIVE,
            PROMPTS,
            ROOT / "scripts" / "listing-prompts.py",
        )
        for path in paths:
            text = path.read_text()
            self.assertNotIn("Sound" + "check", text)
            self.assertNotIn("Pi " + "world", text)
            self.assertNotIn("Wax " + "world", text)
            self.assertNotIn("bg" + "v3", text)
            self.assertNotIn("w" + "ax", text.lower())
            self.assertNotIn("D" + "awn", text)


if __name__ == "__main__":
    unittest.main()
