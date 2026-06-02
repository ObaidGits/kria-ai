from __future__ import annotations

import json
import unittest
from pathlib import Path

from testing.harness.models import RunContext
from testing.harness.runner import (
    load_registry,
    load_suite,
    resolve_selection,
    select_profile_scenarios,
    should_skip,
)


ROOT_DIR = Path(__file__).resolve().parents[3]

PHASE2_SUITES = {
    "rust",
    "ui",
    "playwright",
    "security_audit",
    "release_live",
    "eval_engine",
}


class Phase2SuiteRegistrationTests(unittest.TestCase):
    def _registry(self):
        return load_registry(ROOT_DIR)

    def _context(self, profile: str = "safe") -> RunContext:
        return RunContext(
            root_dir=ROOT_DIR,
            report_dir=ROOT_DIR / "testing" / "eval_reports",
            run_id="unit-test",
            profile=profile,
        )

    def test_phase2_suites_are_registered(self) -> None:
        registry = self._registry()
        self.assertTrue(PHASE2_SUITES.issubset(set(registry)))
        for suite_id in PHASE2_SUITES:
            suite = registry[suite_id]
            self.assertTrue(suite.manifest.exists(), suite_id)
            scenarios = load_suite(suite, ROOT_DIR)
            self.assertGreater(len(scenarios), 0, suite_id)

    def test_phase2_suite_files_have_required_shape(self) -> None:
        for suite_id in PHASE2_SUITES:
            manifest = json.loads(
                (ROOT_DIR / "testing" / "suites" / suite_id / "manifest.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(
                manifest["scenario_files"],
                [
                    f"testing/suites/{suite_id}/scenarios/curated_ci.json",
                    f"testing/suites/{suite_id}/scenarios/generated_inventory.json",
                ],
                suite_id,
            )
            self.assertTrue((ROOT_DIR / "testing" / "suites" / suite_id / "README.md").exists())

    def test_default_phase2_suites_do_not_run_live_slow_or_destructive(self) -> None:
        context = self._context()
        registry = self._registry()
        for suite_id in PHASE2_SUITES:
            runnable = [
                scenario
                for scenario in load_suite(registry[suite_id], ROOT_DIR)
                if should_skip(scenario, context) is None
            ]
            for scenario in runnable:
                self.assertNotIn("live", scenario.tags, scenario.id)
                self.assertNotIn("slow", scenario.tags, scenario.id)
                self.assertNotIn("destructive", scenario.tags, scenario.id)

    def test_all_ci_profile_selects_curated_safe_scenarios_only(self) -> None:
        registry = self._registry()
        _, scenarios = resolve_selection(["all"], registry, ROOT_DIR)
        selected = select_profile_scenarios(scenarios, self._context(profile="ci"))
        selected_ids = {scenario.id for scenario in selected}
        self.assertIn("ui.typecheck", selected_ids)
        self.assertIn("playwright.typecheck", selected_ids)
        self.assertIn("n8n.authoring_validation", selected_ids)
        for scenario in selected:
            self.assertIn("ci", scenario.tags, scenario.id)
            self.assertNotIn("live", scenario.tags, scenario.id)
            self.assertNotIn("slow", scenario.tags, scenario.id)
            self.assertNotIn("destructive", scenario.tags, scenario.id)
            self.assertEqual(scenario.required_services, [], scenario.id)
            self.assertLessEqual(scenario.timeout_seconds, 600, scenario.id)
            self.assertEqual(scenario.cleanup, [], scenario.id)

    def test_inventory_map_reflects_phase2_central_commands(self) -> None:
        inventory = json.loads(
            (ROOT_DIR / "testing" / "inventory" / "current_inventory.json").read_text(
                encoding="utf-8"
            )
        )
        by_path = {entry["path"]: entry for entry in inventory["entries"]}
        expectations = {
            "crates/kria-core/tests/api_unload_model_wiremock.rs": "rust.kria_core_tests_api_unload_model_wiremock",
            "ui/src/components/HitlModal.test.tsx": "ui.vitest.components_hitlmodal_test",
            "testing/suites/playwright/tests/chat.e2e.spec.ts": "playwright.tests_chat_e2e_spec",
            "crates/kria-eval/Cargo.toml": "eval_engine.kria_eval_test",
        }
        for path, scenario_id in expectations.items():
            self.assertIn(path, by_path)
            entry = by_path[path]
            self.assertEqual(entry["centralized_status"], "registered_wrapper", path)
            self.assertIn(f"./testing/run.sh scenario {scenario_id}", entry["central_command"])
            self.assertTrue((ROOT_DIR / path).exists(), path)


if __name__ == "__main__":
    unittest.main()
