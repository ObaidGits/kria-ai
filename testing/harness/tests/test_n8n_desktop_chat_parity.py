from __future__ import annotations

import unittest
from pathlib import Path

from testing.harness.runner import load_registry, load_suite


ROOT_DIR = Path(__file__).resolve().parents[3]

DESKTOP_CHAT_SCENARIOS = {
    "n8n.prompt_e2e.desktop_chat.mock_crud_archive",
    "n8n.prompt_e2e.desktop_chat.command_contract",
}

DESKTOP_COMMAND_SCENARIOS = {
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
}

TAURI_LIVE_SCENARIOS = {
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

PARITY_SCENARIOS = {
    "n8n.prompt_e2e.parity.api_vs_desktop_core",
    "n8n.prompt_e2e.parity.unregistered_n8n_target",
}


class N8nDesktopChatParityTests(unittest.TestCase):
    def _n8n_scenarios(self):
        registry = load_registry(ROOT_DIR)
        return load_suite(registry["n8n"], ROOT_DIR)

    def test_desktop_chat_and_parity_scenarios_are_registered(self) -> None:
        scenarios = {scenario.id: scenario for scenario in self._n8n_scenarios()}
        for scenario_id in DESKTOP_CHAT_SCENARIOS:
            scenario = scenarios[scenario_id]
            self.assertIn("desktop_chat", scenario.tags, scenario_id)
            self.assertIn("prompt_e2e", scenario.tags, scenario_id)
            self.assertIn("live", scenario.tags, scenario_id)
            self.assertIn("slow", scenario.tags, scenario_id)
        for scenario_id in DESKTOP_COMMAND_SCENARIOS:
            scenario = scenarios[scenario_id]
            self.assertEqual(scenario.driver, "desktop_chat_command", scenario_id)
            self.assertIn("desktop_chat", scenario.tags, scenario_id)
            self.assertIn("desktop_command", scenario.tags, scenario_id)
            self.assertIn("prompt_e2e", scenario.tags, scenario_id)
            self.assertIn("live", scenario.tags, scenario_id)
            self.assertIn("slow", scenario.tags, scenario_id)
        for scenario_id in TAURI_LIVE_SCENARIOS:
            scenario = scenarios[scenario_id]
            self.assertIn("desktop_chat", scenario.tags, scenario_id)
            self.assertIn("tauri_live", scenario.tags, scenario_id)
            self.assertIn("prompt_e2e", scenario.tags, scenario_id)
            self.assertIn("live", scenario.tags, scenario_id)
            self.assertIn("slow", scenario.tags, scenario_id)
        for scenario_id in PARITY_SCENARIOS:
            scenario = scenarios[scenario_id]
            self.assertIn("parity", scenario.tags, scenario_id)
            self.assertIn("prompt_e2e", scenario.tags, scenario_id)
            self.assertIn("live", scenario.tags, scenario_id)
            self.assertIn("slow", scenario.tags, scenario_id)

    def test_workflow_hub_smoke_is_not_registered_as_prompt_parity(self) -> None:
        scenarios = {scenario.id: scenario for scenario in self._n8n_scenarios()}
        self.assertIn("n8n.ui_smoke", scenarios)
        self.assertNotIn("n8n.prompt_e2e.native.ui_parity_smoke", scenarios)
        self.assertNotIn("n8n.prompt_e2e.native.ui.create_prompt_parity", scenarios)
        self.assertNotIn("n8n.prompt_e2e.native.ui.archive_prompt_parity", scenarios)

    def test_desktop_chat_spec_uses_real_chat_input_and_send_message(self) -> None:
        spec = (
            ROOT_DIR
            / "testing/suites/playwright/tests/n8n-chat-prompt.tauri-mock.e2e.spec.ts"
        ).read_text(encoding="utf-8")
        self.assertIn('textarea.chat-input', spec)
        self.assertIn('entry.cmd === "send_message"', spec)
        self.assertIn("create_authoring_draft", spec)
        self.assertIn("create_updated_copy", spec)
        self.assertIn("archive_workflow", spec)
        self.assertIn("Lead Capture Automation", spec)
        self.assertIn("GENERIC_N8N_REFUSAL", spec)

    def test_tauri_mock_bridge_emits_agent_stream_for_send_message(self) -> None:
        bridge = (
            ROOT_DIR / "testing/suites/playwright/pages/tauri-mock-bridge.ts"
        ).read_text(encoding="utf-8")
        self.assertIn("chatResponses", bridge)
        self.assertIn("emitEvent(\"agent:token\"", bridge)
        self.assertIn("emitEvent(\"agent:done\"", bridge)

    def test_tauri_live_spec_uses_real_tauri_preflight_without_mock_or_api_chat(self) -> None:
        spec = (
            ROOT_DIR
            / "testing/suites/playwright/tests/n8n-chat-prompt.tauri-live.e2e.spec.ts"
        ).read_text(encoding="utf-8")
        self.assertIn("KRIA_TAURI_LIVE_URL", spec)
        self.assertIn("__TAURI_INTERNALS__", spec)
        self.assertIn("textarea.chat-input", spec)
        self.assertIn("GENERIC_N8N_REFUSAL", spec)
        self.assertNotIn("installTauriMockBridge", spec)
        self.assertNotIn("/api/chat", spec)

    def test_tauri_live_native_driver_is_primary_and_url_mode_is_fallback(self) -> None:
        runner = (
            ROOT_DIR / "testing/suites/n8n/commands/desktop_live_e2e_lib.sh"
        ).read_text(encoding="utf-8")
        self.assertIn('DRIVER_MODE="${KRIA_DESKTOP_LIVE_E2E_DRIVER:-tauri_driver}"', runner)
        self.assertIn("tauri-driver is required", runner)
        self.assertIn("WebKitWebDriver is required", runner)
        self.assertIn("KRIA_TAURI_NATIVE_DRIVER_PATH", runner)
        self.assertIn("KRIA_TAURI_APP_PATH", runner)
        self.assertIn("KRIA_DESKTOP_LIVE_E2E_DRIVER=url", runner)
        self.assertIn("run_tauri_driver crud_archive", runner)
        self.assertIn("exit 78", runner)
        self.assertIn("delete_disposable_workflows_by_prefix", runner)

    def test_tauri_live_driver_auto_registers_disposable_fixture(self) -> None:
        driver = (
            ROOT_DIR
            / "testing/suites/playwright/tauri-live/n8n-desktop-live-driver.mjs"
        ).read_text(encoding="utf-8")
        self.assertIn('"tauri:options"', driver)
        self.assertIn("discover_n8n_runtime_profile_drafts", driver)
        self.assertIn("save_n8n_runtime_profile_draft", driver)
        self.assertIn("save_n8n_profile_as_workflow_draft", driver)
        self.assertIn("remove_n8n_workflow_from_kria", driver)
        self.assertIn("KRIA_DESKTOP_LIVE_E2E_N8N_WORKFLOW_ID", driver)
        self.assertIn("GENERIC_N8N_REFUSAL", driver)
        self.assertIn("workflowFingerprint", driver)
        self.assertIn("n8n_updated_copy_detected", driver)
        self.assertIn("desktop_chat_prompt_failure", driver)
        self.assertIn('"update_exact_copy"', driver)

    def test_desktop_command_driver_is_primary_non_ui_send_message_path(self) -> None:
        driver = (
            ROOT_DIR / "testing/harness/drivers/desktop_chat_command.py"
        ).read_text(encoding="utf-8")
        manifest = (
            ROOT_DIR / "testing/suites/n8n/scenarios/prompt_e2e_desktop_command.json"
        ).read_text(encoding="utf-8")
        rust_chat = (ROOT_DIR / "crates/kria-desktop/src/commands/chat.rs").read_text(encoding="utf-8")
        rust_api = (ROOT_DIR / "crates/kria-desktop/src/commands/local_api.rs").read_text(encoding="utf-8")

        self.assertIn("/api/testing/desktop-chat-command", driver)
        self.assertNotIn("/api/chat", driver)
        self.assertNotIn("playwright", driver.lower())
        self.assertNotIn("tauri-driver", driver.lower())
        self.assertIn('"driver": "desktop_chat_command"', manifest)
        self.assertIn("desktop_n8n_pre_fallback_command_capture", rust_chat)
        self.assertIn("desktop_n8n_pre_fallback_command_capture", rust_api)
        self.assertIn('/api/testing/desktop-chat-command', rust_api)


if __name__ == "__main__":
    unittest.main()
