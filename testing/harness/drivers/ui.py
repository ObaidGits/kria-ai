from __future__ import annotations

from testing.harness.drivers.backend_command import run_backend_command
from testing.harness.models import RunContext, Scenario, ScenarioResult


def run_ui_smoke(scenario: Scenario, context: RunContext) -> ScenarioResult:
    return run_backend_command(scenario, context)

