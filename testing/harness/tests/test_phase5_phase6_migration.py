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
)


ROOT_DIR = Path(__file__).resolve().parents[3]
DECISION_DOC = ROOT_DIR / "testing/inventory/framework_native_decisions.md"
CLEANUP_DOC = ROOT_DIR / "testing/inventory/legacy_cleanup_completed.md"

LEGACY_WRAPPERS = {
    "scripts/" + "run_gui_evals.sh",
    "scripts/" + "build-release.sh",
    "scripts/" + "run_live_stress.sh",
    "scripts/" + "run_release_test_gate.sh",
    "scripts/" + "setup_comfyui.sh",
}


class Phase5Phase6MigrationTests(unittest.TestCase):
    def test_decision_records_exist(self) -> None:
        self.assertTrue(DECISION_DOC.exists())
        self.assertTrue(CLEANUP_DOC.exists())
        decision_text = DECISION_DOC.read_text(encoding="utf-8")
        cleanup_text = CLEANUP_DOC.read_text(encoding="utf-8")
        self.assertIn("crates/*/tests", decision_text)
        self.assertIn("ui/src/**/*.test.*", decision_text)
        self.assertIn("legacy test wrappers removed", cleanup_text.lower())

    def test_rust_and_vitest_tests_remain_framework_native(self) -> None:
        rust_tests = sorted(ROOT_DIR.glob("crates/*/tests/**/*.rs"))
        vitest_tests = sorted(ROOT_DIR.glob("ui/src/**/*.test.*"))
        self.assertGreater(len(rust_tests), 0)
        self.assertGreater(len(vitest_tests), 0)
        self.assertEqual(list((ROOT_DIR / "testing/suites/rust").glob("**/*.rs")), [])
        self.assertEqual(list((ROOT_DIR / "testing/suites/ui").glob("**/*.test.*")), [])

    def test_central_ci_selection_includes_rust_and_ui_coverage(self) -> None:
        registry = load_registry(ROOT_DIR)
        _, scenarios = resolve_selection(["all"], registry, ROOT_DIR)
        selected = select_profile_scenarios(
            scenarios,
            RunContext(
                root_dir=ROOT_DIR,
                report_dir=ROOT_DIR / "testing" / "eval_reports",
                run_id="phase5-phase6-test",
                profile="ci",
            ),
        )
        selected_ids = {scenario.id for scenario in selected}
        self.assertIn("rust.ci.kria_core_api_unload_model_wiremock", selected_ids)
        self.assertIn("rust.ci.kria_core_automation_tests", selected_ids)
        self.assertIn("ui.typecheck", selected_ids)
        for scenario in selected:
            self.assertIn("ci", scenario.tags, scenario.id)
            self.assertNotIn("live", scenario.tags, scenario.id)
            self.assertNotIn("slow", scenario.tags, scenario.id)
            self.assertNotIn("destructive", scenario.tags, scenario.id)

    def test_ui_vitest_all_is_not_ci_in_v1(self) -> None:
        registry = load_registry(ROOT_DIR)
        scenarios = {scenario.id: scenario for scenario in load_suite(registry["ui"], ROOT_DIR)}
        self.assertIn("ui.vitest_all", scenarios)
        self.assertNotIn("ci", scenarios["ui.vitest_all"].tags)

    def test_legacy_wrappers_are_removed(self) -> None:
        wrappers = set(LEGACY_WRAPPERS)
        wrappers.add("scripts/" + "run_all_n8n_checks.sh")
        wrappers.add("scripts/" + "n8n_" + "legacy_notice.sh")
        wrappers.update(
            f"scripts/{path.name}"
            for path in (ROOT_DIR / "testing/suites/n8n/commands").glob("run_n8n_*.sh")
        )
        existing = sorted(path for path in wrappers if (ROOT_DIR / path).exists())
        self.assertEqual(existing, [])

    def test_github_workflows_use_central_testing_commands_for_migrated_areas(self) -> None:
        forbidden = (
            "./scripts/" + "run_n8n_",
            "./scripts/" + "run_all_n8n_checks.sh",
            "./scripts/" + "run_gui_evals.sh",
            "./scripts/" + "run_live_stress.sh",
            "./scripts/" + "run_release_test_gate.sh",
            "./scripts/" + "build-release.sh",
            "./scripts/" + "setup_comfyui.sh",
        )
        for path in (ROOT_DIR / ".github/workflows").glob("*.yml"):
            text = path.read_text(encoding="utf-8")
            for token in forbidden:
                self.assertNotIn(token, text, path.as_posix())

    def test_legacy_pointer_paths_have_no_test_implementation(self) -> None:
        self.assertFalse((ROOT_DIR / "tests" / "e2e").exists())
        self.assertFalse((ROOT_DIR / "tests" / "testing.md").exists())

    def test_inventory_has_no_remove_later_test_wrappers_or_pointers(self) -> None:
        inventory = json.loads(
            (ROOT_DIR / "testing/inventory/current_inventory.json").read_text(encoding="utf-8")
        )
        offenders = [
            entry["path"]
            for entry in inventory["entries"]
            if entry["migration_recommendation"] == "remove_later"
            and (
                entry["path"].startswith("scripts/" + "run_n8n_")
                or entry["path"]
                in {
                    "scripts/" + "run_all_n8n_checks.sh",
                    "scripts/" + "run_gui_evals.sh",
                    "scripts/" + "build-release.sh",
                    "scripts/" + "run_live_stress.sh",
                    "scripts/" + "run_release_test_gate.sh",
                    "scripts/" + "setup_comfyui.sh",
                    "scripts/" + "n8n_" + "legacy_notice.sh",
                    "tests/" + "e2e/README.md",
                    "tests/" + "testing.md",
                }
            )
        ]
        self.assertEqual(offenders, [])

    def test_inventory_marks_playwright_as_centralized_native(self) -> None:
        inventory = json.loads(
            (ROOT_DIR / "testing/inventory/current_inventory.json").read_text(encoding="utf-8")
        )
        by_path = {entry["path"]: entry for entry in inventory["entries"]}
        path = "testing/suites/playwright/tests/chat.e2e.spec.ts"
        self.assertIn(path, by_path)
        self.assertEqual(by_path[path]["migration_recommendation"], "keep_native")


if __name__ == "__main__":
    unittest.main()
