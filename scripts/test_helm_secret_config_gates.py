#!/usr/bin/env python3
"""Contract tests for Helm ConfigMap/Secret/HPA/PVC fail-closed gates."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHART = ROOT / "helm" / "maidan" / "templates"


class HelmSecretConfigGates(unittest.TestCase):
    def test_configmap_refuses_secret_shaped_keys(self) -> None:
        text = (CHART / "configmap.yaml").read_text()
        for needle in (
            'eq $k "DATABASE_URL"',
            'hasSuffix "_SECRET" $k',
            'hasSuffix "_TOKEN" $k',
            'eq $k "MAIDAN_CONTENT_KEK"',
            'fail (printf "config.%s looks like a secret',
            'contains "CHANGE_ME"',
        ):
            self.assertIn(needle, text)

    def test_secret_refuses_placeholders_and_short_kek(self) -> None:
        text = (CHART / "secret.yaml").read_text()
        self.assertIn('contains "CHANGE_ME" (toString $kek)', text)
        self.assertIn('regexMatch "^[0-9a-fA-F]{64}$"', text)
        self.assertIn('regexMatch "^[A-Za-z0-9+/]{43}={0,2}$"', text)
        self.assertIn('contains "CHANGE_ME" (toString $v)', text)
        self.assertIn("contentKekPrevious still holds the placeholder CHANGE_ME", text)

    def test_hpa_bounds_replicas_and_cpu(self) -> None:
        text = (CHART / "hpa.yaml").read_text()
        self.assertIn("lt $min 1", text)
        self.assertIn("lt $max $min", text)
        self.assertIn("gt $max 100", text)
        self.assertIn("targetCPUUtilizationPercentage must be 1..100", text)

    def test_pvc_refuses_empty_or_invalid_size(self) -> None:
        text = (CHART / "pvc.yaml").read_text()
        self.assertIn('eq $size "0Gi"', text)
        self.assertIn("(k|M|G|T|P|E|Ki|Mi|Gi|Ti|Pi|Ei)", text)
        self.assertIn("0?" + chr(92) + chr(92) + ".[0-9]*[1-9]", text)
        self.assertNotIn("(E|P|T|G|M|K|Ei", text)
        self.assertIn("not a valid Kubernetes quantity", text)

    def test_templates_still_render_core_kinds(self) -> None:
        cm = (CHART / "configmap.yaml").read_text()
        self.assertIn("kind: ConfigMap", cm)
        self.assertIn("tpl (toString $v)", cm)
        secret = (CHART / "secret.yaml").read_text()
        self.assertIn("kind: Secret", secret)
        self.assertIn("MAIDAN_CONTENT_KEK:", secret)
        hpa = (CHART / "hpa.yaml").read_text()
        self.assertIn("kind: HorizontalPodAutoscaler", hpa)
        pvc = (CHART / "pvc.yaml").read_text()
        self.assertIn("kind: PersistentVolumeClaim", pvc)

    def test_no_keeper_or_banned_names(self) -> None:
        for path in (
            CHART / "configmap.yaml",
            CHART / "secret.yaml",
            CHART / "hpa.yaml",
            CHART / "pvc.yaml",
        ):
            text = path.read_text()
            self.assertFalse(re.search(r"Soundcheck|Pi world|Wax world|\bwax\b|bgv3|\bDawn\b", text, re.I))


if __name__ == "__main__":
    unittest.main()
