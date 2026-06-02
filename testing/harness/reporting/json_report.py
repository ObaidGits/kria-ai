from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from testing.harness.models import ScenarioResult
from testing.harness.reporting.redaction import redact_json


def build_summary(results: list[ScenarioResult]) -> dict[str, int]:
    summary = {
        "total": len(results),
        "passed": 0,
        "failed": 0,
        "blocked": 0,
        "skipped": 0,
        "infra_failed": 0,
        "cleanup_failed": 0,
    }
    for result in results:
        if result.status in summary:
            summary[result.status] += 1
        elif result.status == "flaky":
            summary["failed"] += 1
    return summary


def overall_status(results: list[ScenarioResult]) -> str:
    if not results:
        return "skipped"
    if any(result.status in {"failed", "cleanup_failed"} for result in results):
        return "failed"
    if any(result.status == "infra_failed" for result in results):
        return "infra_failed"
    if any(result.status == "blocked" for result in results):
        return "blocked"
    if all(result.status == "skipped" for result in results):
        return "skipped"
    return "passed"


def make_report(
    *,
    run_id: str,
    suite_id: str,
    profile: str = "safe",
    selection: dict[str, Any] | None = None,
    started_at_ms: int,
    ended_at_ms: int,
    results: list[ScenarioResult],
) -> dict[str, Any]:
    report = {
        "schema_version": "kria.testing.report.v1",
        "run_id": run_id,
        "suite_id": suite_id,
        "profile": profile,
        "selection": selection or {},
        "started_at_ms": started_at_ms,
        "ended_at_ms": ended_at_ms,
        "status": overall_status(results),
        "summary": build_summary(results),
        "scenarios": [result.__dict__ for result in results],
    }
    return redact_json(report)


def write_json_report(report: dict[str, Any], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
