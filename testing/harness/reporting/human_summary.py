from __future__ import annotations

from pathlib import Path
from typing import Any


def write_markdown_summary(report: dict[str, Any], path: Path) -> None:
    summary = report["summary"]
    lines = [
        f"# KRIA Testing Report: {report['suite_id']}",
        "",
        f"- Run ID: `{report['run_id']}`",
        f"- Profile: `{report.get('profile', 'safe')}`",
        f"- Status: `{report['status']}`",
        f"- Total: `{summary['total']}`",
        f"- Passed: `{summary['passed']}`",
        f"- Failed: `{summary['failed']}`",
        f"- Blocked: `{summary['blocked']}`",
        f"- Skipped: `{summary['skipped']}`",
        f"- Infra failed: `{summary['infra_failed']}`",
        f"- Cleanup failed: `{summary['cleanup_failed']}`",
    ]
    selection = report.get("selection")
    if isinstance(selection, dict) and selection:
        lines.append(f"- Selection policy: `{selection.get('policy', '')}`")
        lines.append(f"- Selected scenarios: `{selection.get('selected', '')}`")
        if selection.get("tag_filters"):
            lines.append(f"- Tag filters: `{', '.join(selection['tag_filters'])}`")
    lines.extend(
        [
            "",
            "| Scenario | Status | Failure class | Duration |",
            "|---|---|---|---:|",
        ]
    )
    for scenario in report["scenarios"]:
        lines.append(
            "| `{}` | `{}` | `{}` | {} ms |".format(
                scenario["scenario_id"],
                scenario["status"],
                scenario.get("failure_class") or "",
                scenario["duration_ms"],
            )
        )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
