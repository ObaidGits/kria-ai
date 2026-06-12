#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import signal
import sys
import time
import uuid
from pathlib import Path
from typing import Any

ROOT_DIR = Path(__file__).resolve().parents[2]
if str(ROOT_DIR) not in sys.path:
    sys.path.insert(0, str(ROOT_DIR))

from testing.harness.cleanup.hooks import run_cleanup_hooks
from testing.harness.drivers.backend_command import run_backend_command
from testing.harness.drivers.chat_api import run_chat_api_scenario
from testing.harness.drivers.desktop_chat_command import run_desktop_chat_command_scenario
from testing.harness.drivers.ui import run_ui_smoke
from testing.harness.env.services import check_required_services
from testing.harness.models import (
    SAFE_DEFAULT_SKIP_TAGS,
    SUPPORTED_DRIVERS,
    SUPPORTED_PROFILES,
    SUPPORTED_SERVICES,
    SUPPORTED_TAGS,
    ManifestError,
    RunContext,
    Scenario,
    ScenarioResult,
    SuiteRef,
)
from testing.harness.reporting.human_summary import write_markdown_summary
from testing.harness.reporting.json_report import make_report, write_json_report


class ScenarioWatchdogTimeout(RuntimeError):
    pass


def now_ms() -> int:
    return int(time.time() * 1000)


def load_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise ManifestError(f"{path}: invalid JSON: {error}") from error


def load_registry(root_dir: Path) -> dict[str, SuiteRef]:
    path = root_dir / "testing" / "registry.json"
    data = load_json(path)
    if data.get("schema_version") != "kria.testing.registry.v1":
        raise ManifestError("registry schema_version must be kria.testing.registry.v1")
    suites: dict[str, SuiteRef] = {}
    for item in data.get("suites", []):
        suite_id = _required_str(item, "id", "registry suite")
        suites[suite_id] = SuiteRef(
            id=suite_id,
            name=_required_str(item, "name", f"registry suite {suite_id}"),
            manifest=root_dir / _required_str(item, "manifest", f"registry suite {suite_id}"),
            default_profile=str(item.get("default_profile", "safe")),
        )
    if not suites:
        raise ManifestError("registry must contain at least one suite")
    return suites


def load_suite(suite: SuiteRef, root_dir: Path) -> list[Scenario]:
    data = load_json(suite.manifest)
    if data.get("schema_version") != "kria.testing.suite.v1":
        raise ManifestError(f"{suite.manifest}: unsupported schema_version")
    if data.get("suite_id") != suite.id:
        raise ManifestError(f"{suite.manifest}: suite_id does not match registry id {suite.id}")
    scenarios: list[Scenario] = []
    for item in data.get("scenarios", []):
        scenarios.append(_scenario_from_dict(item, str(suite.manifest)))
    for scenario_file in data.get("scenario_files", []):
        path = root_dir / scenario_file
        scenario_data = load_json(path)
        for item in scenario_data.get("scenarios", []):
            scenarios.append(_scenario_from_dict(item, str(path)))
    seen: set[str] = set()
    for scenario in scenarios:
        if scenario.id in seen:
            raise ManifestError(f"duplicate scenario id: {scenario.id}")
        seen.add(scenario.id)
    return scenarios


def _required_str(item: dict[str, Any], key: str, context: str) -> str:
    value = item.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ManifestError(f"{context}: missing required string field {key}")
    return value


def _string_list(item: dict[str, Any], key: str, context: str) -> list[str]:
    value = item.get(key, [])
    if not isinstance(value, list) or not all(isinstance(entry, str) for entry in value):
        raise ManifestError(f"{context}: {key} must be a list of strings")
    return value


def _scenario_from_dict(item: dict[str, Any], source: str) -> Scenario:
    scenario_id = _required_str(item, "id", source)
    context = f"{source}:{scenario_id}"
    driver = _required_str(item, "driver", context)
    if driver not in SUPPORTED_DRIVERS:
        raise ManifestError(f"{context}: unsupported driver {driver}")
    tags = _string_list(item, "tags", context)
    unknown_tags = sorted(set(tags) - SUPPORTED_TAGS)
    if unknown_tags:
        raise ManifestError(f"{context}: unsupported tags {unknown_tags}")
    required_services = _string_list(item, "required_services", context)
    unknown_services = sorted(set(required_services) - SUPPORTED_SERVICES)
    if unknown_services:
        raise ManifestError(f"{context}: unsupported services {unknown_services}")
    timeout = item.get("timeout_seconds", 300)
    if not isinstance(timeout, int) or timeout <= 0:
        raise ManifestError(f"{context}: timeout_seconds must be a positive integer")
    return Scenario(
        id=scenario_id,
        title=_required_str(item, "title", context),
        driver=driver,
        tags=tags,
        required_services=required_services,
        timeout_seconds=timeout,
        command=item.get("command"),
        report_artifact_globs=_string_list(item, "report_artifact_globs", context),
        env=item.get("env", {}) if isinstance(item.get("env", {}), dict) else {},
        inputs=item.get("inputs", {}) if isinstance(item.get("inputs", {}), dict) else {},
        assertions=item.get("assertions", []) if isinstance(item.get("assertions", []), list) else [],
        cleanup=item.get("cleanup", []) if isinstance(item.get("cleanup", []), list) else [],
        source_manifest=source,
    )


def should_skip(scenario: Scenario, context: RunContext) -> str | None:
    skip_tags = set()
    if not context.include_live:
        skip_tags.add("live")
    if not context.include_destructive:
        skip_tags.add("destructive")
    if not context.include_slow:
        skip_tags.add("slow")
    matched = sorted(set(scenario.tags) & skip_tags)
    if matched:
        return f"requires explicit flag for tag(s): {', '.join(matched)}"
    if context.tag_filters and not set(context.tag_filters).issubset(set(scenario.tags)):
        return f"does not match tag filter(s): {', '.join(context.tag_filters)}"
    return None


def select_profile_scenarios(scenarios: list[Scenario], context: RunContext) -> list[Scenario]:
    if context.profile == "ci":
        selected = [
            scenario
            for scenario in scenarios
            if "ci" in scenario.tags
            and set(context.tag_filters).issubset(set(scenario.tags))
        ]
        validate_ci_profile_scenarios(selected)
        return selected
    return scenarios


def validate_ci_profile_scenarios(scenarios: list[Scenario]) -> None:
    for scenario in scenarios:
        forbidden = sorted({"live", "slow", "destructive"} & set(scenario.tags))
        if forbidden:
            raise ManifestError(
                f"{scenario.id}: ci profile scenario cannot include tag(s): {', '.join(forbidden)}"
            )
        if scenario.required_services:
            raise ManifestError(
                f"{scenario.id}: ci profile scenario cannot require services: "
                f"{', '.join(scenario.required_services)}"
            )
        if scenario.timeout_seconds > 600:
            raise ManifestError(
                f"{scenario.id}: ci profile timeout must be <= 600 seconds"
            )
        if scenario.cleanup:
            raise ManifestError(f"{scenario.id}: ci profile scenarios cannot require cleanup hooks")


def skipped_result(scenario: Scenario, reason: str) -> ScenarioResult:
    timestamp = now_ms()
    return ScenarioResult(
        scenario_id=scenario.id,
        title=scenario.title,
        status="skipped",
        verdict="skipped",
        failure_class=None,
        started_at_ms=timestamp,
        ended_at_ms=timestamp,
        duration_ms=0,
        tags=scenario.tags,
        required_services=scenario.required_services,
        skip_reason=reason,
    )


def blocked_result(scenario: Scenario, checks: list[Any]) -> ScenarioResult:
    timestamp = now_ms()
    messages = [check.message for check in checks if not check.ok]
    return ScenarioResult(
        scenario_id=scenario.id,
        title=scenario.title,
        status="blocked",
        verdict="blocked",
        failure_class="environment",
        started_at_ms=timestamp,
        ended_at_ms=timestamp,
        duration_ms=0,
        tags=scenario.tags,
        required_services=scenario.required_services,
        evidence=[
            {
                "type": "preflight",
                "checks": [check.__dict__ for check in checks],
            }
        ],
        failure={"message": "; ".join(messages) if messages else "preflight blocked"},
    )


def dry_run_result(scenario: Scenario) -> ScenarioResult:
    timestamp = now_ms()
    return ScenarioResult(
        scenario_id=scenario.id,
        title=scenario.title,
        status="skipped",
        verdict="dry_run",
        failure_class=None,
        started_at_ms=timestamp,
        ended_at_ms=timestamp,
        duration_ms=0,
        tags=scenario.tags,
        required_services=scenario.required_services,
        skip_reason="dry run",
        evidence=[{"type": "dry_run", "driver": scenario.driver, "command": scenario.command}],
    )


def watchdog_timeout_result(
    scenario: Scenario,
    *,
    started_at_ms: int,
    timeout_seconds: int,
) -> ScenarioResult:
    ended_at_ms = now_ms()
    message = (
        f"scenario exceeded hard watchdog after {timeout_seconds}s; "
        "check per-step desktop_chat_command progress evidence for the last request"
    )
    return ScenarioResult(
        scenario_id=scenario.id,
        title=scenario.title,
        status="failed",
        verdict="failed",
        failure_class="harness",
        started_at_ms=started_at_ms,
        ended_at_ms=ended_at_ms,
        duration_ms=ended_at_ms - started_at_ms,
        tags=scenario.tags,
        required_services=scenario.required_services,
        evidence=[
            {
                "type": "scenario_watchdog_timeout",
                "timeout_seconds": timeout_seconds,
                "scenario_timeout_seconds": scenario.timeout_seconds,
            }
        ],
        failure={"message": message},
    )


def execute_scenario_with_watchdog(scenario: Scenario, context: RunContext) -> ScenarioResult:
    started_at_ms = now_ms()
    timeout_seconds = scenario.timeout_seconds + 15
    previous_handler = signal.getsignal(signal.SIGALRM)

    def _handle_timeout(_signum: int, _frame: Any) -> None:
        raise ScenarioWatchdogTimeout()

    signal.signal(signal.SIGALRM, _handle_timeout)
    signal.setitimer(signal.ITIMER_REAL, timeout_seconds)
    try:
        return execute_scenario(scenario, context)
    except ScenarioWatchdogTimeout:
        return watchdog_timeout_result(
            scenario,
            started_at_ms=started_at_ms,
            timeout_seconds=timeout_seconds,
        )
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous_handler)


def execute_scenario(scenario: Scenario, context: RunContext) -> ScenarioResult:
    skip_reason = should_skip(scenario, context)
    if skip_reason:
        return skipped_result(scenario, skip_reason)
    if context.dry_run:
        return dry_run_result(scenario)
    checks = check_required_services(scenario.required_services)
    if any(not check.ok for check in checks):
        return blocked_result(scenario, checks)
    if scenario.driver == "backend_command":
        result = run_backend_command(scenario, context)
    elif scenario.driver == "chat_api":
        result = run_chat_api_scenario(scenario, context)
    elif scenario.driver == "desktop_chat_command":
        result = run_desktop_chat_command_scenario(scenario, context)
    elif scenario.driver == "ui":
        result = run_ui_smoke(scenario, context)
    else:
        result = skipped_result(
            scenario,
            f"{scenario.driver} driver library exists but scenario execution is reserved for a later suite phase",
        )
    cleanup = run_cleanup_hooks(scenario, context)
    if cleanup.get("status") != "not_required":
        result.cleanup = _merge_cleanup(result.cleanup, cleanup)
    if cleanup.get("status") == "failed" and result.status == "passed":
        result.status = "cleanup_failed"
        result.failure_class = "cleanup"
    if result.cleanup.get("status") == "failed" and result.status == "passed":
        result.status = "cleanup_failed"
        result.failure_class = "cleanup"
    return result


def _merge_cleanup(current: dict[str, Any], extra: dict[str, Any]) -> dict[str, Any]:
    current_actions = current.get("actions", []) if isinstance(current, dict) else []
    extra_actions = extra.get("actions", []) if isinstance(extra, dict) else []
    statuses = {
        str(current.get("status") if isinstance(current, dict) else "not_required"),
        str(extra.get("status") if isinstance(extra, dict) else "not_required"),
    }
    if "failed" in statuses:
        status = "failed"
    elif "passed" in statuses:
        status = "passed"
    else:
        status = "not_required"
    return {"status": status, "actions": [*current_actions, *extra_actions]}


def resolve_selection(
    target: list[str], registry: dict[str, SuiteRef], root_dir: Path
) -> tuple[str, list[Scenario]]:
    if not target or target[0] == "all":
        scenarios: list[Scenario] = []
        for suite in registry.values():
            scenarios.extend(load_suite(suite, root_dir))
        return "all", scenarios
    if target[0] == "suite":
        if len(target) != 2:
            raise ManifestError("usage: ./testing/run.sh suite <suite_id>")
        suite = registry.get(target[1])
        if suite is None:
            raise ManifestError(f"unknown suite: {target[1]}")
        return suite.id, load_suite(suite, root_dir)
    if target[0] == "scenario":
        if len(target) != 2:
            raise ManifestError("usage: ./testing/run.sh scenario <scenario_id>")
        scenario_id = target[1]
        for suite in registry.values():
            for scenario in load_suite(suite, root_dir):
                if scenario.id == scenario_id:
                    return scenario.id, [scenario]
        raise ManifestError(f"unknown scenario: {scenario_id}")
    if target[0] in registry:
        suite = registry[target[0]]
        return suite.id, load_suite(suite, root_dir)
    if "." in target[0]:
        return resolve_selection(["scenario", target[0]], registry, root_dir)
    raise ManifestError(f"unknown target: {' '.join(target)}")


def print_list(registry: dict[str, SuiteRef], root_dir: Path) -> None:
    for suite in registry.values():
        print(f"{suite.id}\t{suite.name}")
        for scenario in load_suite(suite, root_dir):
            tags = ",".join(scenario.tags)
            print(f"  {scenario.id}\t{scenario.driver}\t[{tags}]\t{scenario.title}")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="KRIA centralized testing spine")
    parser.add_argument("target", nargs="*", help="all, <suite>, suite <id>, or scenario <id>")
    parser.add_argument("--list", action="store_true", help="list suites and scenarios")
    parser.add_argument("--dry-run", action="store_true", help="show selected scenarios without running")
    parser.add_argument("--include-live", action="store_true", help="include live scenarios")
    parser.add_argument("--include-destructive", action="store_true", help="include destructive scenarios")
    parser.add_argument("--include-slow", action="store_true", help="include slow scenarios")
    parser.add_argument(
        "--profile",
        choices=sorted(SUPPORTED_PROFILES),
        default=None,
        help="selection profile: safe (default) or ci",
    )
    parser.add_argument("--ci", action="store_true", help="shorthand for --profile ci")
    parser.add_argument("--tag", action="append", default=[], help="require tag on selected scenarios")
    parser.add_argument("--fail-fast", action="store_true", help="stop after first failing scenario")
    parser.add_argument("--json", action="store_true", help="print central report JSON path only")
    return parser.parse_args(argv)


def resolve_profile(args: argparse.Namespace) -> str:
    profile = args.profile or "safe"
    if args.ci:
        if args.profile and args.profile != "ci":
            raise ManifestError("--ci cannot be combined with --profile safe")
        profile = "ci"
    if profile not in SUPPORTED_PROFILES:
        raise ManifestError(f"unsupported profile: {profile}")
    if profile == "ci":
        invalid_flags = []
        if args.include_live:
            invalid_flags.append("--include-live")
        if args.include_slow:
            invalid_flags.append("--include-slow")
        if args.include_destructive:
            invalid_flags.append("--include-destructive")
        if invalid_flags:
            raise ManifestError(
                "--profile ci cannot be combined with " + ", ".join(invalid_flags)
            )
    return profile


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    root_dir = ROOT_DIR
    report_dir = root_dir / "testing" / "eval_reports"
    report_dir.mkdir(parents=True, exist_ok=True)
    try:
        profile = resolve_profile(args)
        registry = load_registry(root_dir)
        if args.list:
            print_list(registry, root_dir)
            return 0
        suite_id, scenarios = resolve_selection(args.target, registry, root_dir)
        initial_scenario_count = len(scenarios)
    except ManifestError as error:
        print(f"Manifest error: {error}", file=sys.stderr)
        return 2

    run_suite_id = f"{suite_id}-{profile}" if profile != "safe" else suite_id
    run_id = f"kria-testing-{run_suite_id}-{int(time.time())}-{uuid.uuid4().hex[:8]}"
    context = RunContext(
        root_dir=root_dir,
        report_dir=report_dir,
        run_id=run_id,
        profile=profile,
        include_live=args.include_live,
        include_destructive=args.include_destructive,
        include_slow=args.include_slow,
        tag_filters=args.tag,
        dry_run=args.dry_run,
        fail_fast=args.fail_fast,
    )
    try:
        scenarios = select_profile_scenarios(scenarios, context)
    except ManifestError as error:
        print(f"Manifest error: {error}", file=sys.stderr)
        return 2
    selection = {
        "target": " ".join(args.target) if args.target else "all",
        "profile": profile,
        "policy": "ci_tag_only" if profile == "ci" else "default_skip_tags",
        "initial": initial_scenario_count,
        "selected": len(scenarios),
        "tag_filters": args.tag,
    }
    started_at_ms = now_ms()
    results: list[ScenarioResult] = []
    for scenario in scenarios:
        print(f"[{scenario.id}] {scenario.title}", flush=True)
        result = execute_scenario_with_watchdog(scenario, context)
        print(f"  -> {result.status}", flush=True)
        if result.failure:
            print(f"     {result.failure.get('message')}", flush=True)
        if result.skip_reason:
            print(f"     {result.skip_reason}", flush=True)
        results.append(result)
        if args.fail_fast and result.status in {"failed", "blocked", "infra_failed", "cleanup_failed"}:
            break
    ended_at_ms = now_ms()
    report = make_report(
        run_id=run_id,
        suite_id=suite_id,
        profile=profile,
        selection=selection,
        started_at_ms=started_at_ms,
        ended_at_ms=ended_at_ms,
        results=results,
    )
    timestamp = time.strftime("%Y%m%d_%H%M%S")
    safe_suite_id = run_suite_id.replace(".", "_").replace("-", "_")
    run_suffix = run_id.rsplit("-", 1)[-1]
    report_stem = f"kria_testing_{safe_suite_id}_{timestamp}_{run_suffix}"
    json_path = report_dir / f"{report_stem}.json"
    md_path = report_dir / f"{report_stem}.md"
    write_json_report(report, json_path)
    write_markdown_summary(report, md_path)
    if args.json:
        print(json_path)
    else:
        print(f"Central JSON report: {json_path}")
        print(f"Central summary: {md_path}")
    return 1 if report["status"] in {"failed", "infra_failed", "cleanup_failed"} else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
