from __future__ import annotations

import contextlib
import http.server
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from testing.harness.drivers.desktop_chat_command import (
    run_desktop_chat_command_scenario,
    send_desktop_chat_command,
)
from testing.harness.models import RunContext, Scenario


ROOT_DIR = Path(__file__).resolve().parents[3]


class _SlowDesktopCommandHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self) -> None:  # noqa: N802 - stdlib handler API
        if self.path != "/api/testing/desktop-chat-command":
            self.send_response(404)
            self.end_headers()
            return
        time.sleep(1.0)
        self.send_response(200)
        self.end_headers()
        with contextlib.suppress(OSError):
            self.wfile.write(b'{"status":"ok"}')

    def log_message(self, _format: str, *_args: object) -> None:
        return


class DesktopChatCommandTimeoutTests(unittest.TestCase):
    def test_request_timeout_is_bounded_and_classified(self) -> None:
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _SlowDesktopCommandHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            response = send_desktop_chat_command(
                base_url=f"http://127.0.0.1:{server.server_port}",
                message="Observe my current screen.",
                session_id="timeout-test",
                timeout_seconds=0.1,
            )
        finally:
            server.shutdown()
            server.server_close()

        self.assertFalse(response["ok"])
        self.assertIsNone(response["status_code"])
        self.assertTrue(response["timed_out"])
        self.assertLess(response["duration_ms"], 900)

    def test_scenario_progress_evidence_includes_timeout_and_fixture_context(self) -> None:
        scenario = Scenario(
            id="gui_cognition.timeout.progress",
            title="GUI cognition timeout progress evidence",
            driver="desktop_chat_command",
            tags=["gui_cognition", "desktop_command"],
            required_services=[],
            timeout_seconds=5,
            inputs={
                "base_url": "http://127.0.0.1:3001",
                "manual_profile": {
                    "mode_id": "gui_cognition",
                    "label": "GUI Cognition",
                    "app_lock": "gui_cognition",
                    "tool_lock": None,
                    "strategy": "routed_within_lock",
                },
                "gui_cognition_test": {"llm_planner_fixture": "valid_plan"},
                "steps": [{"prompt": "Observe current screen.", "timeout_seconds": 2}],
                "expected_desktop_path": "send_manual_tool_message",
                "expected_event_names": [],
            },
        )
        with tempfile.TemporaryDirectory() as tmp:
            context = RunContext(root_dir=ROOT_DIR, report_dir=Path(tmp), run_id="unit-run")
            with patch(
                "testing.harness.drivers.desktop_chat_command.health_check",
                return_value={"ok": True, "status_code": 200},
            ), patch(
                "testing.harness.drivers.desktop_chat_command.send_desktop_chat_command",
                return_value={
                    "ok": False,
                    "status_code": None,
                    "duration_ms": 2001,
                    "timed_out": True,
                    "response": {"error": "desktop command request timed out"},
                },
            ):
                result = run_desktop_chat_command_scenario(scenario, context)

        self.assertEqual(result.status, "failed")
        self.assertEqual(result.failure_class, "harness")
        step_evidence = next(item for item in result.evidence if item["type"] == "desktop_chat_command_step")
        self.assertEqual(step_evidence["step_timeout_seconds"], 2)
        self.assertEqual(step_evidence["base_url"], "http://127.0.0.1:3001")
        self.assertEqual(step_evidence["manual_profile_mode_id"], "gui_cognition")
        self.assertEqual(step_evidence["gui_cognition_test"], {"llm_planner_fixture": "valid_plan"})
        self.assertTrue(step_evidence["timed_out"])


if __name__ == "__main__":
    unittest.main()
