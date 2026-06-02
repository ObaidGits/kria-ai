from __future__ import annotations

import unittest
from pathlib import Path

from testing.harness.cleanup.hooks import run_cleanup_hooks
from testing.harness.models import RunContext, Scenario


ROOT_DIR = Path(__file__).resolve().parents[3]


class CleanupHookTests(unittest.TestCase):
    def test_no_cleanup_is_not_required(self) -> None:
        scenario = Scenario(
            id="examples.no_cleanup",
            title="No cleanup",
            driver="backend_command",
            tags=["safe"],
            required_services=[],
        )
        context = RunContext(root_dir=ROOT_DIR, report_dir=ROOT_DIR / "testing" / "eval_reports", run_id="unit-run")
        result = run_cleanup_hooks(scenario, context)
        self.assertEqual(result["status"], "not_required")

    def test_record_only_cleanup_passes(self) -> None:
        scenario = Scenario(
            id="examples.cleanup",
            title="Cleanup",
            driver="backend_command",
            tags=["safe"],
            required_services=[],
            cleanup=[{"kind": "record_only", "message": "noted"}],
        )
        context = RunContext(root_dir=ROOT_DIR, report_dir=ROOT_DIR / "testing" / "eval_reports", run_id="unit-run")
        result = run_cleanup_hooks(scenario, context)
        self.assertEqual(result["status"], "passed")


if __name__ == "__main__":
    unittest.main()

