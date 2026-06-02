from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from testing.harness.models import ManifestError, RunContext, Scenario
from testing.harness.runner import execute_scenario, parse_args, resolve_profile, should_skip


ROOT_DIR = Path(__file__).resolve().parents[3]


class RunnerFakeTests(unittest.TestCase):
    def test_fake_backend_command_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            scenario = Scenario(
                id="examples.fake_pass",
                title="Fake pass",
                driver="backend_command",
                command="printf 'ok\\n'",
                tags=["safe"],
                required_services=[],
                timeout_seconds=5,
            )
            context = RunContext(
                root_dir=ROOT_DIR,
                report_dir=Path(tmp),
                run_id="unit-run",
            )
            result = execute_scenario(scenario, context)
            self.assertEqual(result.status, "passed")

    def test_fake_backend_command_failure_is_product_failure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            scenario = Scenario(
                id="examples.fake_fail",
                title="Fake fail",
                driver="backend_command",
                command="exit 7",
                tags=["safe"],
                required_services=[],
                timeout_seconds=5,
            )
            context = RunContext(
                root_dir=ROOT_DIR,
                report_dir=Path(tmp),
                run_id="unit-run",
            )
            result = execute_scenario(scenario, context)
            self.assertEqual(result.status, "failed")
            self.assertEqual(result.failure_class, "product")

    def test_backend_command_exit_77_is_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            scenario = Scenario(
                id="examples.fake_skip",
                title="Fake skip",
                driver="backend_command",
                command="exit 77",
                tags=["safe"],
                required_services=[],
                timeout_seconds=5,
            )
            context = RunContext(
                root_dir=ROOT_DIR,
                report_dir=Path(tmp),
                run_id="unit-run",
            )
            result = execute_scenario(scenario, context)
            self.assertEqual(result.status, "skipped")
            self.assertIn("exit 77", result.skip_reason or "")

    def test_backend_command_suppresses_legacy_n8n_script_notice(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            scenario = Scenario(
                id="examples.legacy_notice_env",
                title="Legacy notice env",
                driver="backend_command",
                command='test "${KRIA_SUPPRESS_LEGACY_N8N_NOTICE:-}" = "1"',
                tags=["safe"],
                required_services=[],
                timeout_seconds=5,
            )
            context = RunContext(
                root_dir=ROOT_DIR,
                report_dir=Path(tmp),
                run_id="unit-run",
            )
            result = execute_scenario(scenario, context)
            self.assertEqual(result.status, "passed")

    def test_live_scenario_skips_by_default(self) -> None:
        scenario = Scenario(
            id="examples.live",
            title="Live",
            driver="backend_command",
            command="true",
            tags=["safe", "live"],
            required_services=[],
        )
        context = RunContext(root_dir=ROOT_DIR, report_dir=ROOT_DIR / "testing" / "eval_reports", run_id="unit-run")
        self.assertIn("live", should_skip(scenario, context) or "")

    def test_ci_flag_resolves_to_ci_profile(self) -> None:
        args = parse_args(["n8n", "--ci"])
        self.assertEqual(resolve_profile(args), "ci")

    def test_ci_profile_rejects_live_slow_and_destructive_flags(self) -> None:
        for flag in ("--include-live", "--include-slow", "--include-destructive"):
            with self.subTest(flag=flag):
                args = parse_args(["n8n", "--profile", "ci", flag])
                with self.assertRaises(ManifestError):
                    resolve_profile(args)

    def test_ci_shorthand_cannot_conflict_with_safe_profile(self) -> None:
        args = parse_args(["n8n", "--profile", "safe", "--ci"])
        with self.assertRaises(ManifestError):
            resolve_profile(args)


if __name__ == "__main__":
    unittest.main()
