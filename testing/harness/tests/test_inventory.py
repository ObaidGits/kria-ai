from __future__ import annotations

import json
import subprocess
import unittest
from pathlib import Path

from testing.tools.collect_test_inventory import (
    EXCLUDED_PARTS,
    INVENTORY_JSON,
    MIGRATION_MAP,
    ROOT_DIR,
    validate_inventory,
)


class TestingInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.inventory = json.loads(INVENTORY_JSON.read_text(encoding="utf-8"))
        cls.entries = cls.inventory["entries"]
        cls.by_path = {entry["path"]: entry for entry in cls.entries}
        cls.migration_map = MIGRATION_MAP.read_text(encoding="utf-8")

    def test_inventory_json_exists_and_matches_schema(self) -> None:
        self.assertTrue(INVENTORY_JSON.exists())
        self.assertFalse(validate_inventory(self.inventory))
        self.assertEqual(self.inventory["entry_count"], len(self.entries))

    def test_inventory_check_command_passes(self) -> None:
        completed = subprocess.run(
            ["python3", "testing/tools/collect_test_inventory.py", "--check"],
            cwd=ROOT_DIR,
            text=True,
            capture_output=True,
            timeout=30,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_every_central_n8n_command_script_is_present(self) -> None:
        expected = {
            path.relative_to(ROOT_DIR).as_posix()
            for path in (ROOT_DIR / "testing/suites/n8n/commands").glob("run_n8n_*.sh")
        }
        expected.add("testing/suites/n8n/commands/run_all_n8n_checks.sh")
        missing = sorted(expected - set(self.by_path))
        self.assertEqual(missing, [])
        for path in expected:
            self.assertEqual(self.by_path[path]["suite_group"], "n8n")

    def test_old_n8n_wrapper_scripts_are_absent_from_inventory(self) -> None:
        offenders = [
            path
            for path in self.by_path
            if path.startswith("scripts/" + "run_n8n_")
            or path == "scripts/" + "run_all_n8n_checks.sh"
        ]
        self.assertEqual(offenders, [])

    def test_rust_tests_are_framework_native(self) -> None:
        paths = [
            path.relative_to(ROOT_DIR).as_posix()
            for path in ROOT_DIR.glob("crates/*/tests/**/*.rs")
            if "__pycache__" not in path.parts
        ]
        self.assertGreater(len(paths), 0)
        for path in paths:
            self.assertIn(path, self.by_path)
            self.assertTrue(self.by_path[path]["framework_native"], path)
            self.assertEqual(self.by_path[path]["kind"], "rust_test")

    def test_vitest_tests_are_framework_native(self) -> None:
        paths = [
            path.relative_to(ROOT_DIR).as_posix()
            for path in ROOT_DIR.glob("ui/src/**/*.test.*")
        ]
        self.assertGreater(len(paths), 0)
        for path in paths:
            self.assertIn(path, self.by_path)
            self.assertTrue(self.by_path[path]["framework_native"], path)
            self.assertEqual(self.by_path[path]["kind"], "vitest_test")

    def test_playwright_tests_are_centralized_native_assets(self) -> None:
        paths = [
            path.relative_to(ROOT_DIR).as_posix()
            for path in ROOT_DIR.glob("testing/suites/playwright/tests/**/*.spec.ts")
        ]
        self.assertGreater(len(paths), 0)
        for path in paths:
            self.assertIn(path, self.by_path)
            entry = self.by_path[path]
            self.assertEqual(entry["kind"], "playwright_test")
            self.assertEqual(entry["migration_recommendation"], "keep_native")

    def test_no_phase1_deletes_are_allowed(self) -> None:
        offenders = [
            entry["path"]
            for entry in self.entries
            if entry["delete_allowed_phase1"] is not False
        ]
        self.assertEqual(offenders, [])

    def test_live_and_destructive_items_are_not_marked_safe(self) -> None:
        for entry in self.entries:
            if entry["safety"] in {"live", "destructive"}:
                self.assertNotEqual(entry["safety"], "safe", entry["path"])

    def test_generated_cache_paths_are_excluded(self) -> None:
        offenders = [
            entry["path"]
            for entry in self.entries
            if any(part in EXCLUDED_PARTS for part in entry["path"].split("/"))
        ]
        self.assertEqual(offenders, [])

    def test_github_workflows_do_not_call_old_n8n_scripts(self) -> None:
        for path in (ROOT_DIR / ".github/workflows").glob("*.yml"):
            text = path.read_text(encoding="utf-8")
            self.assertNotIn("./scripts/" + "run_n8n_", text, path.as_posix())
            self.assertNotIn("./scripts/" + "run_all_n8n_checks.sh", text, path.as_posix())

    def test_existing_n8n_scenarios_map_to_registered_scripts(self) -> None:
        scenario_files = list((ROOT_DIR / "testing/suites/n8n/scenarios").glob("*.json"))
        commands: set[str] = set()
        for path in scenario_files:
            data = json.loads(path.read_text(encoding="utf-8"))
            for scenario in data.get("scenarios", []):
                command = scenario.get("command")
                if isinstance(command, str) and command.startswith("./testing/suites/n8n/commands/"):
                    commands.add(command.removeprefix("./"))
        for command in sorted(commands):
            self.assertTrue((ROOT_DIR / command).exists(), command)

    def test_unknown_classifications_are_visible_in_migration_map(self) -> None:
        unknown_paths = [
            entry["path"]
            for entry in self.entries
            if entry["safety"] == "unknown" or entry["suite_group"] == "unknown"
        ]
        self.assertGreater(len(unknown_paths), 0)
        for path in unknown_paths:
            self.assertIn(path, self.migration_map)


if __name__ == "__main__":
    unittest.main()
