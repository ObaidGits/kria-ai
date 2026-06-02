from __future__ import annotations

import unittest
from pathlib import Path

from testing.harness.models import ManifestError
from testing.harness.runner import _scenario_from_dict, load_registry, load_suite


ROOT_DIR = Path(__file__).resolve().parents[3]


class ManifestValidationTests(unittest.TestCase):
    def test_repo_registry_loads_n8n_suite(self) -> None:
        registry = load_registry(ROOT_DIR)
        self.assertIn("n8n", registry)
        scenarios = load_suite(registry["n8n"], ROOT_DIR)
        self.assertTrue(any(scenario.id == "n8n.authoring_validation" for scenario in scenarios))

    def test_invalid_driver_fails_validation(self) -> None:
        with self.assertRaises(ManifestError):
            _scenario_from_dict(
                {
                    "id": "bad.driver",
                    "title": "bad",
                    "driver": "unknown",
                    "tags": ["safe"],
                    "required_services": [],
                    "timeout_seconds": 1,
                },
                "unit-test",
            )

    def test_invalid_tag_fails_validation(self) -> None:
        with self.assertRaises(ManifestError):
            _scenario_from_dict(
                {
                    "id": "bad.tag",
                    "title": "bad",
                    "driver": "backend_command",
                    "tags": ["surprise"],
                    "required_services": [],
                    "timeout_seconds": 1,
                    "command": "true",
                },
                "unit-test",
            )


if __name__ == "__main__":
    unittest.main()

