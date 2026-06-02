from __future__ import annotations

import unittest
from pathlib import Path

from testing.harness.models import RunContext, SUPPORTED_SERVICES
from testing.harness.runner import load_registry, load_suite, select_profile_scenarios, should_skip


ROOT_DIR = Path(__file__).resolve().parents[3]


EXPECTED_N8N_SCENARIO_IDS = {
    "n8n.testing_spine_self_tests",
    "n8n.phase0_contract",
    "n8n.runtime_modes",
    "n8n.phase2_ui_contract",
    "n8n.phase3_progress",
    "n8n.phase4_management",
    "n8n.phase5_invocation",
    "n8n.phase6_readiness_gate",
    "n8n.routing_baseline",
    "n8n.chat_routing_eval",
    "n8n.stage3_routing_eval",
    "n8n.authoring_validation",
    "n8n.ui_smoke",
    "n8n.production_audit",
    "n8n.full_capability_eval",
    "n8n.basic_prompt_eval",
    "n8n.live_e2e_callback",
    "n8n.stage2_6_catalog_e2e",
    "n8n.webhook_polling_smoke",
    "n8n.reliability_tests",
    "n8n.prompt_e2e",
    "n8n.authoring_live_smoke",
    "n8n.all_checks_aggregate",
    "n8n.prompt_e2e.native.inventory_list",
    "n8n.prompt_e2e.native.non_n8n_no_hijack",
    "n8n.prompt_e2e.native.create_http_movie_lookup",
    "n8n.prompt_e2e.native.safe_delete_offers_archive",
    "n8n.prompt_e2e.native.archive_restore_disposable",
    "n8n.prompt_e2e.native.permanent_delete_danger_only",
    "n8n.prompt_e2e.native.update_creates_copy",
    "n8n.prompt_e2e.native.credential_missing_blocker",
    "n8n.prompt_e2e.native.unsupported_destructive_request",
    "n8n.prompt_e2e.native.leftover_cleanup_check",
    "n8n.prompt_e2e.native.drift_blocks_run",
}

EXPECTED_DEFAULT_N8N_SCENARIO_IDS = {
    "n8n.testing_spine_self_tests",
    "n8n.phase0_contract",
    "n8n.runtime_modes",
    "n8n.phase2_ui_contract",
    "n8n.phase3_progress",
    "n8n.phase4_management",
    "n8n.phase5_invocation",
    "n8n.routing_baseline",
    "n8n.chat_routing_eval",
    "n8n.stage3_routing_eval",
    "n8n.authoring_validation",
}

EXPECTED_CI_N8N_SCENARIO_IDS = {
    "n8n.testing_spine_self_tests",
    "n8n.phase0_contract",
    "n8n.runtime_modes",
    "n8n.phase2_ui_contract",
    "n8n.phase3_progress",
    "n8n.phase4_management",
    "n8n.phase5_invocation",
    "n8n.chat_routing_eval",
    "n8n.stage3_routing_eval",
    "n8n.authoring_validation",
}

EXPECTED_ROUTING_SCENARIO_IDS = {
    "n8n.routing_baseline",
    "n8n.chat_routing_eval",
    "n8n.stage3_routing_eval",
}

EXPECTED_SLOW_SCENARIO_IDS = {
    "n8n.phase6_readiness_gate",
    "n8n.ui_smoke",
    "n8n.production_audit",
    "n8n.full_capability_eval",
    "n8n.basic_prompt_eval",
    "n8n.live_e2e_callback",
    "n8n.stage2_6_catalog_e2e",
    "n8n.prompt_e2e",
    "n8n.prompt_e2e.native.inventory_list",
    "n8n.prompt_e2e.native.non_n8n_no_hijack",
    "n8n.prompt_e2e.native.create_http_movie_lookup",
    "n8n.prompt_e2e.native.safe_delete_offers_archive",
    "n8n.prompt_e2e.native.archive_restore_disposable",
    "n8n.prompt_e2e.native.permanent_delete_danger_only",
    "n8n.prompt_e2e.native.update_creates_copy",
    "n8n.prompt_e2e.native.credential_missing_blocker",
    "n8n.prompt_e2e.native.unsupported_destructive_request",
    "n8n.prompt_e2e.native.leftover_cleanup_check",
    "n8n.prompt_e2e.native.drift_blocks_run",
    "n8n.all_checks_aggregate",
}

EXPECTED_NATIVE_PROMPT_E2E_SCENARIO_IDS = {
    "n8n.prompt_e2e.native.inventory_list",
    "n8n.prompt_e2e.native.non_n8n_no_hijack",
    "n8n.prompt_e2e.native.create_http_movie_lookup",
    "n8n.prompt_e2e.native.safe_delete_offers_archive",
    "n8n.prompt_e2e.native.archive_restore_disposable",
    "n8n.prompt_e2e.native.permanent_delete_danger_only",
    "n8n.prompt_e2e.native.update_creates_copy",
    "n8n.prompt_e2e.native.credential_missing_blocker",
    "n8n.prompt_e2e.native.unsupported_destructive_request",
    "n8n.prompt_e2e.native.leftover_cleanup_check",
    "n8n.prompt_e2e.native.drift_blocks_run",
    "n8n.prompt_e2e.native.inventory.show_archived",
    "n8n.prompt_e2e.native.inventory.runnable_workflows",
    "n8n.prompt_e2e.native.routing.github_no_hijack",
    "n8n.prompt_e2e.native.create.schedule_draft",
    "n8n.prompt_e2e.native.create.slack_credential_blocker",
    "n8n.prompt_e2e.native.run.exact_approved",
    "n8n.prompt_e2e.native.run.draft_blocked",
    "n8n.prompt_e2e.native.update.ambiguous_clarifies",
    "n8n.prompt_e2e.native.update.direct_original_danger_only",
    "n8n.prompt_e2e.native.lifecycle.drift_blocks_run",
    "n8n.prompt_e2e.native.lifecycle.missing_copy_recovery",
    "n8n.prompt_e2e.native.cleanup.cleanup_only",
    "n8n.prompt_e2e.native.v5.file_required_missing_file",
    "n8n.prompt_e2e.native.v5.multipart_fixture",
    "n8n.prompt_e2e.native.output.high_confidence_selected",
    "n8n.prompt_e2e.native.output.low_confidence_requires_choice",
    "n8n.prompt_e2e.native.credentials.no_secret_leak",
    "n8n.prompt_e2e.native.hitl.yellow_confirm_required",
    "n8n.prompt_e2e.native.hitl.red_blocked",
    "n8n.prompt_e2e.desktop_chat.mock_crud_archive",
    "n8n.prompt_e2e.desktop_chat.command_contract",
    "n8n.prompt_e2e.parity.api_vs_desktop_core",
    "n8n.prompt_e2e.parity.unregistered_n8n_target",
    "n8n.desktop_command.list_workflows",
    "n8n.desktop_command.create_http_movie_lookup",
    "n8n.desktop_command.update_exact_copy",
    "n8n.desktop_command.safe_delete_archive_offer",
    "n8n.desktop_command.archive_workflow",
    "n8n.desktop_command.restore_workflow",
    "n8n.desktop_command.permanent_delete_danger_only",
    "n8n.desktop_command.unregistered_target_blocker",
    "n8n.desktop_command.non_n8n_no_hijack",
    "n8n.desktop_command.cleanup_leftover_detector",
    "n8n.prompt_e2e.desktop_live.crud_archive",
    "n8n.desktop_live.create_http_movie_lookup",
    "n8n.desktop_live.list_workflows",
    "n8n.desktop_live.update_exact_copy",
    "n8n.desktop_live.safe_delete_archive_offer",
    "n8n.desktop_live.archive_workflow",
    "n8n.desktop_live.restore_workflow",
    "n8n.desktop_live.permanent_delete_danger_only",
    "n8n.desktop_live.unregistered_target_blocker",
    "n8n.desktop_live.non_n8n_no_hijack",
    "n8n.desktop_live.cleanup_leftover_detector",
    "n8n.prompt_e2e.desktop_live.unregistered_target",
    "n8n.prompt_e2e.desktop_live.cleanup",
}

EXPECTED_N8N_SCENARIO_IDS.update(
    EXPECTED_NATIVE_PROMPT_E2E_SCENARIO_IDS - EXPECTED_N8N_SCENARIO_IDS
)
EXPECTED_SLOW_SCENARIO_IDS.update(
    EXPECTED_NATIVE_PROMPT_E2E_SCENARIO_IDS - EXPECTED_SLOW_SCENARIO_IDS
)


class N8nSuiteExpansionTests(unittest.TestCase):
    def _n8n_scenarios(self):
        registry = load_registry(ROOT_DIR)
        return load_suite(registry["n8n"], ROOT_DIR)

    def _default_context(self, tag_filters: list[str] | None = None) -> RunContext:
        return RunContext(
            root_dir=ROOT_DIR,
            report_dir=ROOT_DIR / "testing" / "eval_reports",
            run_id="unit-test",
            tag_filters=tag_filters or [],
        )

    def _ci_context(self, tag_filters: list[str] | None = None) -> RunContext:
        return RunContext(
            root_dir=ROOT_DIR,
            report_dir=ROOT_DIR / "testing" / "eval_reports",
            run_id="unit-test",
            profile="ci",
            tag_filters=tag_filters or [],
        )

    def test_n8n_scenarios_are_registered_once(self) -> None:
        scenarios = self._n8n_scenarios()
        scenario_ids = [scenario.id for scenario in scenarios]
        self.assertEqual(len(scenario_ids), len(set(scenario_ids)))
        self.assertEqual(set(scenario_ids), EXPECTED_N8N_SCENARIO_IDS)

    def test_all_n8n_scenarios_have_n8n_tag_and_existing_scripts(self) -> None:
        for scenario in self._n8n_scenarios():
            self.assertIn("n8n", scenario.tags, scenario.id)
            self.assertIn(scenario.driver, {"backend_command", "chat_api", "desktop_chat_command"}, scenario.id)
            command = scenario.command or ""
            if command.startswith("./"):
                command_path = ROOT_DIR / command.removeprefix("./")
                self.assertTrue(command_path.exists(), f"{scenario.id}: missing {command}")

    def test_n8n_legacy_wrappers_are_removed_and_registered_commands_exist(self) -> None:
        registered_scripts = {
            Path(scenario.command.removeprefix("./")).name
            for scenario in self._n8n_scenarios()
            if (scenario.command or "").startswith("./testing/suites/n8n/commands/")
        }

        expected_scripts = {
            path.name
            for path in (ROOT_DIR / "testing/suites/n8n/commands").glob("run_n8n_*.sh")
        }
        expected_scripts.add("run_all_n8n_checks.sh")

        self.assertEqual(expected_scripts, registered_scripts)
        self.assertEqual(len(expected_scripts), 35)
        self.assertFalse((ROOT_DIR / "scripts" / "run_all_n8n_checks.sh").exists())
        self.assertEqual(list(ROOT_DIR.glob("scripts/run_n8n_*.sh")), [])
        self.assertFalse((ROOT_DIR / "scripts" / ("n8n_" + "legacy_notice.sh")).exists())
        for script_name in sorted(expected_scripts):
            command_text = (ROOT_DIR / "testing/suites/n8n/commands" / script_name).read_text()
            self.assertNotIn("kria_" + "n8n_" + "legacy_notice", command_text, script_name)
            self.assertNotIn("scripts/" + "n8n_" + "legacy_notice.sh", command_text, script_name)

    def test_stale_wrapped_existing_scenario_file_is_absent(self) -> None:
        self.assertFalse(
            (ROOT_DIR / "testing/suites/n8n/scenarios/wrapped_existing.json").exists()
        )

    def test_github_workflows_use_central_n8n_commands(self) -> None:
        forbidden = ("./scripts/" + "run_n8n_", "./scripts/" + "run_all_n8n_checks.sh")
        for path in (ROOT_DIR / ".github/workflows").glob("*.yml"):
            text = path.read_text()
            for token in forbidden:
                self.assertNotIn(token, text, str(path))

    def test_live_ui_slow_and_required_service_classification(self) -> None:
        for scenario in self._n8n_scenarios():
            if scenario.id in {
                "n8n.full_capability_eval",
                "n8n.basic_prompt_eval",
                "n8n.live_e2e_callback",
                "n8n.stage2_6_catalog_e2e",
                "n8n.webhook_polling_smoke",
                "n8n.reliability_tests",
                "n8n.prompt_e2e",
                "n8n.authoring_live_smoke",
                *EXPECTED_NATIVE_PROMPT_E2E_SCENARIO_IDS,
            }:
                self.assertIn("live", scenario.tags, scenario.id)
            if "ui" in scenario.tags:
                self.assertIn("ui", scenario.tags, scenario.id)
            if scenario.id in EXPECTED_SLOW_SCENARIO_IDS:
                self.assertIn("slow", scenario.tags, scenario.id)
            for service in scenario.required_services:
                self.assertIn(service, SUPPORTED_SERVICES, scenario.id)

    def test_default_n8n_selection_excludes_live_slow_and_destructive(self) -> None:
        context = self._default_context()
        selected = {
            scenario.id
            for scenario in self._n8n_scenarios()
            if should_skip(scenario, context) is None
        }
        self.assertEqual(selected, EXPECTED_DEFAULT_N8N_SCENARIO_IDS)

    def test_ci_profile_selects_only_curated_ci_scenarios(self) -> None:
        scenarios = select_profile_scenarios(self._n8n_scenarios(), self._ci_context())
        selected = {scenario.id for scenario in scenarios}
        self.assertEqual(selected, EXPECTED_CI_N8N_SCENARIO_IDS)
        for scenario in scenarios:
            self.assertIn("ci", scenario.tags, scenario.id)
            self.assertNotIn("live", scenario.tags, scenario.id)
            self.assertNotIn("slow", scenario.tags, scenario.id)
            self.assertNotIn("destructive", scenario.tags, scenario.id)
            self.assertEqual(scenario.required_services, [], scenario.id)
            self.assertLessEqual(scenario.timeout_seconds, 600, scenario.id)
            self.assertEqual(scenario.cleanup, [], scenario.id)

    def test_ci_profile_tag_filter_refines_selected_scenarios(self) -> None:
        scenarios = select_profile_scenarios(
            self._n8n_scenarios(),
            self._ci_context(tag_filters=["routing"]),
        )
        selected = {scenario.id for scenario in scenarios}
        self.assertEqual(selected, {"n8n.chat_routing_eval", "n8n.stage3_routing_eval"})

    def test_routing_tag_selects_only_routing_scenarios(self) -> None:
        context = self._default_context(tag_filters=["routing"])
        selected = {
            scenario.id
            for scenario in self._n8n_scenarios()
            if should_skip(scenario, context) is None
        }
        self.assertEqual(selected, EXPECTED_ROUTING_SCENARIO_IDS)
        for scenario in self._n8n_scenarios():
            if scenario.id in selected:
                self.assertIn("routing", scenario.tags)

    def test_prompt_e2e_tag_selects_native_scenarios_when_live_and_slow_are_enabled(self) -> None:
        context = RunContext(
            root_dir=ROOT_DIR,
            report_dir=ROOT_DIR / "testing" / "eval_reports",
            run_id="unit-test",
            include_live=True,
            include_slow=True,
            tag_filters=["prompt_e2e"],
        )
        selected = {
            scenario.id
            for scenario in self._n8n_scenarios()
            if should_skip(scenario, context) is None
        }
        self.assertEqual(selected, EXPECTED_NATIVE_PROMPT_E2E_SCENARIO_IDS)


if __name__ == "__main__":
    unittest.main()
