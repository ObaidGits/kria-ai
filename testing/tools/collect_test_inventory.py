#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

ROOT_DIR = Path(__file__).resolve().parents[2]
INVENTORY_DIR = ROOT_DIR / "testing" / "inventory"
INVENTORY_JSON = INVENTORY_DIR / "current_inventory.json"
MIGRATION_MAP = INVENTORY_DIR / "migration_map.md"

SCHEMA_VERSION = "kria.testing.inventory.v1"

ENTRY_REQUIRED_FIELDS = {
    "path",
    "kind",
    "suite_guess",
    "suite_group",
    "runner",
    "safety",
    "framework_native",
    "current_command",
    "central_command",
    "centralized_status",
    "migration_recommendation",
    "delete_allowed_phase1",
    "notes",
}

KINDS = {
    "shell_script",
    "rust_test",
    "vitest_test",
    "playwright_test",
    "python_test",
    "docs",
    "workflow",
    "harness_test",
    "eval_engine",
}
SUITE_GUESSES = {
    "n8n",
    "ui",
    "rust",
    "hitl",
    "memory",
    "voice",
    "security_audit",
    "release_gate",
    "docs",
    "unknown",
}
SUITE_GROUPS = {
    "n8n",
    "rust",
    "ui_vitest",
    "ui_playwright",
    "hitl",
    "memory",
    "voice",
    "security_audit",
    "release_gate",
    "docs",
    "harness",
    "eval_engine",
    "unknown",
}
RUNNERS = {
    "bash",
    "cargo",
    "vitest",
    "playwright",
    "python_unittest",
    "pytest",
    "github_actions",
    "docs_only",
    "cargo_eval",
    "unknown",
}
SAFETY = {"safe", "live", "destructive", "unknown"}
CENTRALIZED_STATUS = {
    "native",
    "registered_wrapper",
    "legacy_unregistered",
    "framework_native_unregistered",
    "docs_only",
}
MIGRATION_RECOMMENDATIONS = {
    "keep_native",
    "register_only",
    "move_body_to_testing",
    "move_directory",
    "wrapper_later",
    "remove_later",
}

EXCLUDED_PARTS = {
    ".git",
    "target",
    "node_modules",
    "__pycache__",
    ".pytest_cache",
    "test-results",
    "playwright-report",
    "blob-report",
    "dist",
    "eval_reports",
}

DESTRUCTIVE_HINTS = (
    "kria_dangerous",
    "dangerous",
    "destructive",
    "shutdown",
    "git push",
    "delete real",
    "rm -rf",
)
LIVE_HINTS = (
    "live",
    "real_llm",
    "real llm",
    "voice_live",
    "docker",
    "n8n api",
    "api key",
    "kria_api",
    "browser",
    "playwright",
    "webhook",
    "callback",
    "localhost",
    "127.0.0.1",
)
SAFE_HINTS = (
    "static",
    "contract",
    "dry-run",
    "dry run",
    "unit",
    "vitest",
    "cargo test",
    "unittest",
)


def rel(path: Path) -> str:
    return path.relative_to(ROOT_DIR).as_posix()


def should_exclude(path: Path) -> bool:
    try:
        relative = path.relative_to(ROOT_DIR)
    except ValueError:
        return True
    return any(part in EXCLUDED_PARTS for part in relative.parts)


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return ""


def load_central_command_map() -> dict[str, list[dict[str, Any]]]:
    mapping: dict[str, list[dict[str, Any]]] = defaultdict(list)
    registry_path = ROOT_DIR / "testing" / "registry.json"
    if not registry_path.exists():
        return mapping
    registry = json.loads(registry_path.read_text(encoding="utf-8"))
    for suite in registry.get("suites", []):
        manifest_path = ROOT_DIR / str(suite.get("manifest", ""))
        if not manifest_path.exists():
            continue
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        scenario_sources = [manifest]
        for scenario_file in manifest.get("scenario_files", []):
            path = ROOT_DIR / scenario_file
            if path.exists():
                scenario_sources.append(json.loads(path.read_text(encoding="utf-8")))
        for source in scenario_sources:
            for scenario in source.get("scenarios", []):
                command = scenario.get("command")
                if isinstance(command, str) and command:
                    mapping[command].append(
                        {
                            "id": scenario.get("id", ""),
                            "tags": scenario.get("tags", []),
                            "suite": suite.get("id", ""),
                        }
                    )
    return mapping


def central_command_for_scenario(scenario: dict[str, Any]) -> str:
    command = f"./testing/run.sh scenario {scenario['id']}"
    tags = set(scenario.get("tags", []))
    if "live" in tags:
        command += " --include-live"
    if "slow" in tags:
        command += " --include-slow"
    if "destructive" in tags:
        command += " --include-destructive"
    return command


def choose_script_scenario(path: str, scenarios: list[dict[str, Any]]) -> dict[str, Any] | None:
    if not scenarios:
        return None
    if path.endswith("run_n8n_ui_smoke.sh"):
        for scenario in scenarios:
            if scenario.get("id") == "n8n.ui_smoke":
                return scenario
    non_prompt = [
        scenario
        for scenario in scenarios
        if ".prompt_e2e.native." not in str(scenario.get("id", ""))
    ]
    non_ci = [
        scenario
        for scenario in non_prompt
        if ".ci." not in str(scenario.get("id", ""))
    ]
    candidates = non_ci or non_prompt or scenarios
    return sorted(candidates, key=lambda item: str(item.get("id", "")))[0]


def detect_kind(path: Path) -> str:
    path_str = rel(path)
    name = path.name
    if path_str.startswith("testing/harness/tests/") and name.endswith(".py"):
        return "harness_test"
    if path_str.startswith("kria-modules/tests/") and name.endswith(".py"):
        return "python_test"
    if "/tests/" in path_str and name.endswith(".rs"):
        return "rust_test"
    if path_str.startswith("ui/src/") and ".test." in name:
        return "vitest_test"
    if path_str.startswith("testing/suites/playwright/") and name.endswith(".spec.ts"):
        return "playwright_test"
    if path_str.startswith(".github/workflows/") and name.endswith((".yml", ".yaml")):
        return "workflow"
    if path_str.startswith("docs/evaluations/") or path_str.startswith("testing/docs/"):
        return "docs"
    if path_str.startswith("crates/kria-eval/"):
        return "eval_engine"
    if path_str.startswith("scripts/") and name.endswith(".sh"):
        return "shell_script"
    return "docs"


def detect_runner(kind: str, path: Path) -> str:
    if kind == "shell_script":
        return "bash"
    if kind == "rust_test":
        return "cargo"
    if kind == "vitest_test":
        return "vitest"
    if kind == "playwright_test":
        return "playwright"
    if kind == "harness_test":
        return "python_unittest"
    if kind == "python_test":
        return "pytest"
    if kind == "workflow":
        return "github_actions"
    if kind == "eval_engine":
        return "cargo_eval"
    return "docs_only"


def detect_suite(path: Path, kind: str, text: str) -> tuple[str, str]:
    path_str = rel(path).lower()
    combined = f"{path_str}\n{text.lower()}"
    if path_str.startswith("testing/suites/n8n/"):
        return "n8n", "n8n"
    if path_str.startswith("testing/suites/eval_engine/"):
        return "unknown", "eval_engine"
    if path_str.startswith("testing/suites/release_live/"):
        return "release_gate", "release_gate"
    if kind == "vitest_test":
        return "ui", "ui_vitest"
    if kind == "playwright_test":
        return "ui", "ui_playwright"
    if kind == "harness_test":
        return "unknown", "harness"
    if kind in {"docs", "workflow"}:
        return "docs", "docs"
    if kind == "rust_test":
        if "voice" in combined:
            return "voice", "voice"
        if "memory" in combined:
            return "memory", "memory"
        if "hitl" in combined:
            return "hitl", "hitl"
        if any(token in combined for token in ("safety", "policy", "dangerous", "audit")):
            return "security_audit", "security_audit"
        return "rust", "rust"
    if kind == "eval_engine":
        if "hitl" in combined:
            return "hitl", "hitl"
        return "unknown", "eval_engine"
    if "n8n" in combined:
        return "n8n", "n8n"
    if "release" in combined or "stress" in combined:
        return "release_gate", "release_gate"
    if "voice" in combined:
        return "voice", "voice"
    if "memory" in combined:
        return "memory", "memory"
    if "hitl" in combined:
        return "hitl", "hitl"
    if any(token in combined for token in ("security", "safety", "audit", "dangerous")):
        return "security_audit", "security_audit"
    return "unknown", "unknown"


def detect_safety(path: Path, kind: str, text: str) -> str:
    lower = f"{rel(path).lower()}\n{text.lower()}"
    if any(hint in lower for hint in DESTRUCTIVE_HINTS):
        return "destructive"
    if kind == "playwright_test":
        return "live"
    if any(hint in lower for hint in LIVE_HINTS):
        return "live"
    if kind in {"docs", "workflow", "harness_test", "vitest_test"}:
        return "safe"
    if any(hint in lower for hint in SAFE_HINTS):
        return "safe"
    if kind == "rust_test":
        return "safe"
    return "unknown"


def is_framework_native(kind: str, path: Path) -> bool:
    path_str = rel(path)
    return (
        kind in {"rust_test", "vitest_test", "harness_test", "eval_engine", "python_test"}
        or path_str.startswith("crates/")
        or path_str.startswith("ui/src/")
    )


def current_command(kind: str, path: Path) -> str:
    path_str = rel(path)
    name = path.name
    if kind == "shell_script":
        return f"./{path_str}"
    if kind == "rust_test":
        parts = path_str.split("/")
        crate = parts[1] if len(parts) > 2 else ""
        if len(parts) == 4 and parts[2] == "tests":
            return f"cargo test -p {crate} --test {Path(name).stem}"
        return f"cargo test -p {crate}"
    if kind == "vitest_test":
        return f"cd ui && npm run test:run -- {Path(name).stem}"
    if kind == "playwright_test":
        return (
            "cd testing/suites/playwright && npx playwright test "
            f"{path_str.removeprefix('testing/suites/playwright/')}"
        )
    if kind == "harness_test":
        return "PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover testing/harness/tests"
    if kind == "python_test":
        return "pytest kria-modules/tests"
    if kind == "eval_engine":
        return "cargo test -p kria-eval"
    return ""


def central_status_and_command(
    path: Path,
    kind: str,
    suite_group: str,
    central_scripts: dict[str, list[dict[str, Any]]],
) -> tuple[str, str, list[str]]:
    notes: list[str] = []
    path_str = rel(path)
    command_key = current_command(kind, path)
    scenarios = central_scripts.get(command_key, [])
    scenario = choose_script_scenario(path_str, scenarios)
    if scenario:
        if len(scenarios) > 1:
            ids = ", ".join(sorted(str(item.get("id", "")) for item in scenarios))
            notes.append(f"Registered by multiple scenarios: {ids}")
        return "registered_wrapper", central_command_for_scenario(scenario), notes
    if kind == "harness_test":
        return "native", "PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover testing/harness/tests", notes
    if kind in {"rust_test", "vitest_test", "python_test", "eval_engine"}:
        return "framework_native_unregistered", f"pending central suite for {suite_group}", notes
    if kind == "playwright_test":
        return "legacy_unregistered", "pending: ./testing/run.sh suite ui --tag playwright", notes
    if kind in {"docs", "workflow"}:
        return "docs_only", "", notes
    return "legacy_unregistered", "", notes


def migration_recommendation(kind: str, path: Path, status: str, suite_group: str) -> str:
    path_str = rel(path)
    if status == "native":
        return "keep_native"
    if path_str.startswith("testing/suites/") and "/commands/" in path_str:
        return "keep_native"
    if path_str.startswith("testing/suites/playwright/") and kind == "playwright_test":
        return "keep_native"
    if kind == "shell_script":
        return "register_only"
    if kind in {"rust_test", "vitest_test", "python_test", "eval_engine"}:
        return "keep_native"
    if kind == "playwright_test":
        return "move_directory"
    if kind in {"docs", "workflow"}:
        return "register_only"
    return "register_only"


def inventory_paths() -> list[Path]:
    paths: set[Path] = set()

    for pattern in (
        "scripts/*.sh",
        "testing/suites/*/commands/*.sh",
        "testing/harness/tests/*.py",
        "testing/suites/playwright/tests/**/*.spec.ts",
        "ui/src/**/*.test.*",
        "crates/*/tests/**/*.rs",
        "kria-modules/tests/**/*.py",
        "crates/kria-eval/Cargo.toml",
        "crates/kria-eval/src/**/*.rs",
        "docs/evaluations/**/*.md",
        ".github/workflows/*.yml",
    ):
        for path in ROOT_DIR.glob(pattern):
            if path.is_file() and not should_exclude(path):
                paths.add(path)

    for doc in (ROOT_DIR / "testing" / "docs" / "legacy-testing.md",):
        if doc.exists():
            paths.add(doc)

    return sorted(paths, key=rel)


def collect_inventory() -> dict[str, Any]:
    central_scripts = load_central_command_map()
    entries: list[dict[str, Any]] = []
    for path in inventory_paths():
        text = read_text(path)
        kind = detect_kind(path)
        suite_guess, suite_group = detect_suite(path, kind, text)
        status, central_command, notes = central_status_and_command(
            path, kind, suite_group, central_scripts
        )
        safety = detect_safety(path, kind, text)
        if status == "registered_wrapper":
            if "--include-destructive" in central_command:
                safety = "destructive"
            elif "--include-live" in central_command:
                safety = "live"
            else:
                safety = "safe"
        entry = {
            "path": rel(path),
            "kind": kind,
            "suite_guess": suite_guess,
            "suite_group": suite_group,
            "runner": detect_runner(kind, path),
            "safety": safety,
            "framework_native": is_framework_native(kind, path),
            "current_command": current_command(kind, path),
            "central_command": central_command,
            "centralized_status": status,
            "migration_recommendation": migration_recommendation(kind, path, status, suite_group),
            "delete_allowed_phase1": False,
            "notes": notes,
        }
        entries.append(entry)

    return {
        "schema_version": SCHEMA_VERSION,
        "generated_by": "testing/tools/collect_test_inventory.py",
        "entry_count": len(entries),
        "summary": {
            "by_kind": dict(sorted(Counter(entry["kind"] for entry in entries).items())),
            "by_safety": dict(sorted(Counter(entry["safety"] for entry in entries).items())),
            "by_suite_group": dict(sorted(Counter(entry["suite_group"] for entry in entries).items())),
            "by_centralized_status": dict(
                sorted(Counter(entry["centralized_status"] for entry in entries).items())
            ),
        },
        "entries": entries,
    }


def validate_inventory(data: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if data.get("schema_version") != SCHEMA_VERSION:
        errors.append(f"schema_version must be {SCHEMA_VERSION}")
    entries = data.get("entries")
    if not isinstance(entries, list):
        errors.append("entries must be a list")
        return errors
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            errors.append(f"entries[{index}] must be an object")
            continue
        missing = sorted(ENTRY_REQUIRED_FIELDS - set(entry))
        if missing:
            errors.append(f"{entry.get('path', index)}: missing fields {missing}")
            continue
        _validate_enum(errors, entry, "kind", KINDS)
        _validate_enum(errors, entry, "suite_guess", SUITE_GUESSES)
        _validate_enum(errors, entry, "suite_group", SUITE_GROUPS)
        _validate_enum(errors, entry, "runner", RUNNERS)
        _validate_enum(errors, entry, "safety", SAFETY)
        _validate_enum(errors, entry, "centralized_status", CENTRALIZED_STATUS)
        _validate_enum(errors, entry, "migration_recommendation", MIGRATION_RECOMMENDATIONS)
        if not isinstance(entry["framework_native"], bool):
            errors.append(f"{entry['path']}: framework_native must be bool")
        if entry["delete_allowed_phase1"] is not False:
            errors.append(f"{entry['path']}: delete_allowed_phase1 must be false")
        if not isinstance(entry["notes"], list):
            errors.append(f"{entry['path']}: notes must be a list")
        for field in ("path", "current_command", "central_command"):
            if not isinstance(entry[field], str):
                errors.append(f"{entry['path']}: {field} must be a string")
        if any(part in entry["path"].split("/") for part in EXCLUDED_PARTS):
            errors.append(f"{entry['path']}: excluded generated/cache path included")
    return errors


def _validate_enum(errors: list[str], entry: dict[str, Any], field: str, allowed: set[str]) -> None:
    if entry[field] not in allowed:
        errors.append(f"{entry.get('path', '<unknown>')}: invalid {field}={entry[field]}")


def write_inventory(data: dict[str, Any]) -> None:
    INVENTORY_DIR.mkdir(parents=True, exist_ok=True)
    INVENTORY_JSON.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    MIGRATION_MAP.write_text(render_migration_map(data), encoding="utf-8")


def render_migration_map(data: dict[str, Any]) -> str:
    entries = data["entries"]
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for entry in entries:
        grouped[entry["suite_group"]].append(entry)

    lines = [
        "# KRIA Testing Migration Map",
        "",
        "Generated by `testing/tools/collect_test_inventory.py`.",
        "",
        "Phase 1 is inventory-only: no test, script, or documentation path is moved or deleted.",
        "",
        "## Summary",
        "",
        f"- Total entries: {data['entry_count']}",
    ]
    for key, values in data["summary"].items():
        formatted = ", ".join(f"{name}: {count}" for name, count in values.items())
        lines.append(f"- {key}: {formatted}")
    lines.extend(["", "## Groups", ""])

    group_order = [
        "n8n",
        "rust",
        "ui_vitest",
        "ui_playwright",
        "hitl",
        "memory",
        "voice",
        "security_audit",
        "release_gate",
        "docs",
        "harness",
        "eval_engine",
        "unknown",
    ]
    for group in group_order:
        items = sorted(grouped.get(group, []), key=lambda entry: entry["path"])
        if not items:
            continue
        lines.extend(
            [
                f"### {group}",
                "",
                "| Path | Current command | Central command | Safety | Native | Recommendation | Delete in Phase 1 |",
                "| --- | --- | --- | --- | ---: | --- | ---: |",
            ]
        )
        for entry in items:
            lines.append(
                "| {path} | {current} | {central} | {safety} | {native} | {recommendation} | {delete} |".format(
                    path=_md(entry["path"]),
                    current=_md(entry["current_command"] or "-"),
                    central=_md(entry["central_command"] or "-"),
                    safety=entry["safety"],
                    native="yes" if entry["framework_native"] else "no",
                    recommendation=entry["migration_recommendation"],
                    delete="yes" if entry["delete_allowed_phase1"] else "no",
                )
            )
        unknowns = [
            entry
            for entry in items
            if entry["safety"] == "unknown" or entry["suite_guess"] == "unknown"
        ]
        if unknowns:
            lines.extend(["", "Unknown/review-required entries:"])
            for entry in unknowns:
                lines.append(f"- `{entry['path']}`")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def _md(value: str) -> str:
    escaped = value.replace("|", "\\|")
    return f"`{escaped}`" if escaped != "-" else "-"


def check_inventory() -> int:
    collected = collect_inventory()
    errors = validate_inventory(collected)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    if not INVENTORY_JSON.exists():
        print(f"missing inventory file: {INVENTORY_JSON}", file=sys.stderr)
        return 1
    existing = json.loads(INVENTORY_JSON.read_text(encoding="utf-8"))
    if existing != collected:
        print("current_inventory.json is stale; run collect_test_inventory.py --write", file=sys.stderr)
        return 1
    expected_map = render_migration_map(collected)
    if not MIGRATION_MAP.exists() or MIGRATION_MAP.read_text(encoding="utf-8") != expected_map:
        print("migration_map.md is stale; run collect_test_inventory.py --write", file=sys.stderr)
        return 1
    print(f"inventory ok: {collected['entry_count']} entries")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Collect KRIA test migration inventory")
    parser.add_argument("--write", action="store_true", help="write inventory JSON and migration map")
    parser.add_argument("--check", action="store_true", help="validate committed inventory is current")
    args = parser.parse_args()

    if args.check:
        return check_inventory()

    data = collect_inventory()
    errors = validate_inventory(data)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    if args.write:
        write_inventory(data)
        print(f"wrote {INVENTORY_JSON}")
        print(f"wrote {MIGRATION_MAP}")
    else:
        print(json.dumps(data, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
