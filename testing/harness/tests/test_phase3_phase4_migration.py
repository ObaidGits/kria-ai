from __future__ import annotations

import json
import unittest
from pathlib import Path

from testing.harness.runner import load_registry, load_suite


ROOT_DIR = Path(__file__).resolve().parents[3]

REMOVED_LEGACY_WRAPPERS = {
    "scripts/" + "run_gui_evals.sh",
    "scripts/" + "build-release.sh",
    "scripts/" + "run_live_stress.sh",
    "scripts/" + "run_release_test_gate.sh",
    "scripts/" + "setup_comfyui.sh",
    "scripts/" + "run_all_n8n_checks.sh",
}


class Phase3Phase4MigrationTests(unittest.TestCase):
    def test_central_manifests_do_not_call_migrated_legacy_script_bodies(self) -> None:
        forbidden = (
            "./scripts/" + "run_n8n_",
            "./scripts/" + "run_all_n8n_checks.sh",
            "./scripts/" + "run_gui_evals.sh",
            "./scripts/" + "run_live_stress.sh",
            "./scripts/" + "run_release_test_gate.sh",
            "./scripts/" + "build-release.sh",
            "./scripts/" + "setup_comfyui.sh",
        )
        registry = load_registry(ROOT_DIR)
        for suite_id in {"n8n", "eval_engine", "release_live", "playwright"}:
            for scenario in load_suite(registry[suite_id], ROOT_DIR):
                command = scenario.command or ""
                for token in forbidden:
                    self.assertNotIn(token, command, scenario.id)

    def test_legacy_wrappers_are_removed_and_central_commands_exist(self) -> None:
        removed = set(REMOVED_LEGACY_WRAPPERS)
        removed.update(
            f"scripts/{path.name}"
            for path in (ROOT_DIR / "testing/suites/n8n/commands").glob("run_n8n_*.sh")
        )

        for wrapper in sorted(removed):
            self.assertFalse((ROOT_DIR / wrapper).exists(), wrapper)

        self.assertTrue((ROOT_DIR / "testing/suites/eval_engine/commands/run_gui_evals.sh").exists())
        for name in (
            "build-release.sh",
            "run_live_stress.sh",
            "run_release_test_gate.sh",
            "setup_comfyui.sh",
        ):
            self.assertTrue((ROOT_DIR / "testing/suites/release_live/commands" / name).exists())
        self.assertTrue((ROOT_DIR / "testing/suites/n8n/commands/run_all_n8n_checks.sh").exists())

    def test_migrated_command_scripts_do_not_call_old_wrappers(self) -> None:
        forbidden = (
            "$ROOT_DIR/scripts/" + "run_n8n_",
            "./scripts/" + "run_n8n_",
            "$ROOT_DIR/scripts/" + "run_all_n8n_checks.sh",
            "./scripts/" + "run_all_n8n_checks.sh",
        )
        for path in (ROOT_DIR / "testing/suites/n8n/commands").glob("*.sh"):
            text = path.read_text(encoding="utf-8")
            for token in forbidden:
                self.assertNotIn(token, text, path.as_posix())

    def test_playwright_sources_live_under_testing_suite(self) -> None:
        self.assertTrue((ROOT_DIR / "testing/suites/playwright/package.json").exists())
        self.assertTrue((ROOT_DIR / "testing/suites/playwright/playwright.config.ts").exists())
        specs = sorted((ROOT_DIR / "testing/suites/playwright/tests").glob("*.spec.ts"))
        self.assertGreater(len(specs), 0)
        self.assertFalse((ROOT_DIR / "tests" / "e2e").exists())

    def test_playwright_scenarios_use_central_path(self) -> None:
        registry = load_registry(ROOT_DIR)
        for scenario in load_suite(registry["playwright"], ROOT_DIR):
            command = scenario.command or ""
            self.assertNotIn("tests" + "/e2e", command, scenario.id)
            if "playwright" in command or "npm run" in command:
                self.assertIn("testing/suites/playwright", command, scenario.id)

    def test_inventory_maps_migrated_paths(self) -> None:
        inventory = json.loads(
            (ROOT_DIR / "testing/inventory/current_inventory.json").read_text(
                encoding="utf-8"
            )
        )
        by_path = {entry["path"]: entry for entry in inventory["entries"]}
        self.assertIn("testing/suites/playwright/tests/chat.e2e.spec.ts", by_path)
        self.assertNotIn("tests" + "/e2e/tests/chat.e2e.spec.ts", by_path)
        self.assertIn("testing/docs/legacy-testing.md", by_path)

    def test_github_workflows_do_not_use_old_playwright_working_directory(self) -> None:
        for path in (ROOT_DIR / ".github/workflows").glob("*.yml"):
            text = path.read_text(encoding="utf-8")
            self.assertNotIn("working-directory: " + "tests" + "/e2e", text, path.as_posix())
            self.assertNotIn("cd ../" + "tests" + "/e2e", text, path.as_posix())


if __name__ == "__main__":
    unittest.main()
