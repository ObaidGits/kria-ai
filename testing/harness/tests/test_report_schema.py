from __future__ import annotations

import json
import unittest
from pathlib import Path

from testing.harness.models import ScenarioResult
from testing.harness.reporting.json_report import make_report


ROOT_DIR = Path(__file__).resolve().parents[3]


class ReportSchemaTests(unittest.TestCase):
    def test_report_contains_required_fields(self) -> None:
        result = ScenarioResult(
            scenario_id="examples.fake",
            title="Fake",
            status="passed",
            verdict="passed",
            failure_class=None,
            started_at_ms=1,
            ended_at_ms=2,
            duration_ms=1,
            tags=["safe"],
            required_services=[],
        )
        report = make_report(
            run_id="run-1",
            suite_id="examples",
            profile="ci",
            selection={
                "target": "examples",
                "profile": "ci",
                "policy": "ci_tag_only",
                "selected": 1,
            },
            started_at_ms=1,
            ended_at_ms=2,
            results=[result],
        )
        self.assertEqual(report["schema_version"], "kria.testing.report.v1")
        self.assertEqual(report["profile"], "ci")
        self.assertEqual(report["selection"]["policy"], "ci_tag_only")
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["summary"]["passed"], 1)
        self.assertEqual(report["scenarios"][0]["scenario_id"], "examples.fake")

    def test_schema_files_are_valid_json(self) -> None:
        for path in (ROOT_DIR / "testing" / "schemas").glob("*.json"):
            with self.subTest(path=path):
                json.loads(path.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
