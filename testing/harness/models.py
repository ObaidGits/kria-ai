from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


SAFE_DEFAULT_SKIP_TAGS = {"live", "destructive", "slow"}
SUPPORTED_PROFILES = {"safe", "ci"}
SUPPORTED_DRIVERS = {"backend_command", "chat_api", "desktop_chat_command", "ui", "n8n_api"}
SUPPORTED_SERVICES = {"kria_api", "n8n", "docker", "browser"}
SUPPORTED_TAGS = {
    "aggregate",
    "build",
    "ci",
    "safe",
    "live",
    "llm_planner",
    "step3_planner",
    "step4_validator",
    "step5_target_resolver",
    "step6_safety_hitl",
    "step7_executor",
    "step8_verification",
    "step9_recovery",
    "step10_workflow_runtime",
    "step11_checkpoint_resume",
    "step12_real_task_eval",
    "destructive",
    "desktop_chat",
    "desktop_command",
    "ui",
    "api",
    "eval_engine",
    "gui_cognition",
    "goal_intent",
    "goal_contract",
    "n8n",
    "audit",
    "authoring",
    "callback",
    "capability",
    "catalog",
    "cleanup",
    "credentials",
    "file",
    "hitl",
    "invocation",
    "legacy",
    "lifecycle",
    "management",
    "memory",
    "output",
    "phase",
    "playwright",
    "prompt_e2e",
    "parity",
    "readiness",
    "reliability",
    "release",
    "routing",
    "backend_route",
    "action_backend",
    "active_window",
    "focus_authority",
    "control_fusion",
    "visual_controls",
    "accessibility_health",
    "ocr_performance",
    "perception_completion",
    "execution",
    "context_builder",
    "perception",
    "smoke",
    "security",
    "regression",
    "slow",
    "stress",
    "tauri_live",
    "tauri_mock",
    "rust",
    "typecheck",
    "v5",
    "vitest",
    "voice",
    "workspace",
}
REPORT_STATUSES = {
    "passed",
    "failed",
    "blocked",
    "skipped",
    "infra_failed",
    "flaky",
    "cleanup_failed",
}
FAILURE_CLASSES = {
    "product",
    "environment",
    "harness",
    "assertion",
    "cleanup",
    "unknown",
}


class ManifestError(ValueError):
    pass


@dataclass
class SuiteRef:
    id: str
    name: str
    manifest: Path
    default_profile: str = "safe"


@dataclass
class Scenario:
    id: str
    title: str
    driver: str
    tags: list[str]
    required_services: list[str]
    timeout_seconds: int = 300
    command: str | None = None
    report_artifact_globs: list[str] = field(default_factory=list)
    env: dict[str, str] = field(default_factory=dict)
    inputs: dict[str, Any] = field(default_factory=dict)
    assertions: list[dict[str, Any]] = field(default_factory=list)
    cleanup: list[dict[str, Any]] = field(default_factory=list)
    source_manifest: str | None = None


@dataclass
class ServiceCheck:
    service: str
    ok: bool
    message: str


@dataclass
class ScenarioResult:
    scenario_id: str
    title: str
    status: str
    verdict: str
    failure_class: str | None
    started_at_ms: int
    ended_at_ms: int
    duration_ms: int
    tags: list[str]
    required_services: list[str]
    evidence: list[dict[str, Any]] = field(default_factory=list)
    artifacts: list[str] = field(default_factory=list)
    skip_reason: str | None = None
    failure: dict[str, Any] | None = None
    cleanup: dict[str, Any] = field(
        default_factory=lambda: {"status": "not_required", "actions": []}
    )


@dataclass
class RunContext:
    root_dir: Path
    report_dir: Path
    run_id: str
    profile: str = "safe"
    include_live: bool = False
    include_destructive: bool = False
    include_slow: bool = False
    tag_filters: list[str] = field(default_factory=list)
    dry_run: bool = False
    fail_fast: bool = False
