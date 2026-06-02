from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from testing.harness.assertions.chat_response import assert_chat_response
from testing.harness.drivers.n8n_api import is_disposable_workflow_name
from testing.harness.models import RunContext, Scenario
from testing.harness.runner import execute_scenario


ROOT_DIR = Path(__file__).resolve().parents[3]


class ChatApiDriverTests(unittest.TestCase):
    def test_chat_response_blocks_generic_create_refusal(self) -> None:
        failures = assert_chat_response(
            {
                "status": "received",
                "reply": "I don't have a tool available that can create or modify n8n workflows.",
            },
            {"fail_on_generic_create_refusal": True},
        )
        self.assertTrue(any("generic" in failure for failure in failures))

    def test_chat_response_accepts_expected_n8n_action(self) -> None:
        failures = assert_chat_response(
            {
                "status": "draft_created",
                "reply": "Inactive n8n draft created.",
                "n8n": {"action": "create_authoring_draft"},
            },
            {
                "expected_status_any": ["draft_created"],
                "expected_n8n_action": "create_authoring_draft",
                "expected_reply_contains_any": ["draft"],
            },
        )
        self.assertEqual(failures, [])

    def test_n8n_delete_guard_allows_only_disposable_prefixes(self) -> None:
        self.assertTrue(is_disposable_workflow_name("KRIA E2E Test run"))
        self.assertFalse(is_disposable_workflow_name("Customer billing workflow"))

    def test_chat_api_scenario_executes_with_mocked_local_api(self) -> None:
        scenario = Scenario(
            id="n8n.prompt_e2e.native.mock",
            title="Mock native prompt E2E",
            driver="chat_api",
            tags=["live", "slow", "n8n", "api", "prompt_e2e"],
            required_services=[],
            timeout_seconds=10,
            inputs={
                "prompt": "Create an n8n workflow",
                "expected_status_any": ["draft_created"],
                "expected_n8n_action": "create_authoring_draft",
                "fail_on_generic_create_refusal": True,
            },
        )
        with tempfile.TemporaryDirectory() as tmp:
            context = RunContext(
                root_dir=ROOT_DIR,
                report_dir=Path(tmp),
                run_id="unit-run",
                include_live=True,
                include_slow=True,
            )
            with patch(
                "testing.harness.drivers.chat_api.health_check",
                return_value={"ok": True, "status_code": 200},
            ), patch(
                "testing.harness.drivers.chat_api.send_chat_message",
                return_value={
                    "ok": True,
                    "status_code": 200,
                    "duration_ms": 5,
                    "response": {
                        "status": "draft_created",
                        "reply": "Inactive n8n draft created.",
                        "n8n": {"action": "create_authoring_draft"},
                    },
                },
            ):
                result = execute_scenario(scenario, context)
        self.assertEqual(result.status, "passed")


if __name__ == "__main__":
    unittest.main()
