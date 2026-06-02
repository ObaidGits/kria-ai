from __future__ import annotations

import glob
import os
import subprocess
import time
from pathlib import Path
from typing import Any

from testing.harness.models import RunContext, Scenario, ScenarioResult
from testing.harness.reporting.redaction import redact_text


def _now_ms() -> int:
    return int(time.time() * 1000)


def _collect_artifacts(root_dir: Path, patterns: list[str], started_at: float) -> list[str]:
    artifacts: list[str] = []
    for pattern in patterns:
        for match in glob.glob(str(root_dir / pattern)):
            path = Path(match)
            try:
                if path.is_file() and path.stat().st_mtime >= started_at - 1:
                    artifacts.append(str(path))
            except OSError:
                continue
    return sorted(set(artifacts))


def run_backend_command(scenario: Scenario, context: RunContext) -> ScenarioResult:
    started_at = time.time()
    started_ms = _now_ms()
    command = scenario.command
    if not command:
        ended_ms = _now_ms()
        return ScenarioResult(
            scenario_id=scenario.id,
            title=scenario.title,
            status="failed",
            verdict="failed",
            failure_class="harness",
            started_at_ms=started_ms,
            ended_at_ms=ended_ms,
            duration_ms=ended_ms - started_ms,
            tags=scenario.tags,
            required_services=scenario.required_services,
            failure={"message": "backend_command scenario is missing command"},
        )

    env = os.environ.copy()
    env.update(scenario.env)
    env.setdefault("KRIA_SUPPRESS_LEGACY_N8N_NOTICE", "1")
    env["REPORT_DIR"] = str(context.report_dir)

    try:
        completed = subprocess.run(
            command,
            cwd=context.root_dir,
            env=env,
            shell=True,
            text=True,
            capture_output=True,
            timeout=scenario.timeout_seconds,
        )
        ended_ms = _now_ms()
        artifacts = _collect_artifacts(
            context.root_dir, scenario.report_artifact_globs, started_at
        )
        evidence: list[dict[str, Any]] = [
            {
                "type": "process",
                "command": command,
                "exit_code": completed.returncode,
                "stdout_preview": redact_text(completed.stdout, limit=4000),
                "stderr_preview": redact_text(completed.stderr, limit=4000),
            }
        ]
        if completed.returncode == 0:
            return ScenarioResult(
                scenario_id=scenario.id,
                title=scenario.title,
                status="passed",
                verdict="passed",
                failure_class=None,
                started_at_ms=started_ms,
                ended_at_ms=ended_ms,
                duration_ms=ended_ms - started_ms,
                tags=scenario.tags,
                required_services=scenario.required_services,
                evidence=evidence,
                artifacts=artifacts,
            )
        if completed.returncode == 77:
            return ScenarioResult(
                scenario_id=scenario.id,
                title=scenario.title,
                status="skipped",
                verdict="skipped",
                failure_class=None,
                started_at_ms=started_ms,
                ended_at_ms=ended_ms,
                duration_ms=ended_ms - started_ms,
                tags=scenario.tags,
                required_services=scenario.required_services,
                evidence=evidence,
                artifacts=artifacts,
                skip_reason="command requested skip with exit 77",
            )
        if completed.returncode == 78:
            return ScenarioResult(
                scenario_id=scenario.id,
                title=scenario.title,
                status="blocked",
                verdict="blocked",
                failure_class="environment",
                started_at_ms=started_ms,
                ended_at_ms=ended_ms,
                duration_ms=ended_ms - started_ms,
                tags=scenario.tags,
                required_services=scenario.required_services,
                evidence=evidence,
                artifacts=artifacts,
                failure={
                    "message": "command requested environment block with exit 78",
                    "exit_code": completed.returncode,
                },
            )
        return ScenarioResult(
            scenario_id=scenario.id,
            title=scenario.title,
            status="failed",
            verdict="failed",
            failure_class="product",
            started_at_ms=started_ms,
            ended_at_ms=ended_ms,
            duration_ms=ended_ms - started_ms,
            tags=scenario.tags,
            required_services=scenario.required_services,
            evidence=evidence,
            artifacts=artifacts,
            failure={
                "message": f"command exited with status {completed.returncode}",
                "exit_code": completed.returncode,
            },
        )
    except subprocess.TimeoutExpired as error:
        ended_ms = _now_ms()
        return ScenarioResult(
            scenario_id=scenario.id,
            title=scenario.title,
            status="infra_failed",
            verdict="infra_failed",
            failure_class="environment",
            started_at_ms=started_ms,
            ended_at_ms=ended_ms,
            duration_ms=ended_ms - started_ms,
            tags=scenario.tags,
            required_services=scenario.required_services,
            failure={"message": f"command timed out after {scenario.timeout_seconds}s"},
            evidence=[
                {
                    "type": "process_timeout",
                    "command": command,
                    "stdout_preview": redact_text(error.stdout or "", limit=2000),
                    "stderr_preview": redact_text(error.stderr or "", limit=2000),
                }
            ],
        )
