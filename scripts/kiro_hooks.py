#!/usr/bin/env python3
"""Resource-aware validation dispatcher for KRIA Kiro Hooks.

Uses only Python's standard library. Checks are sequential, lock-protected, and
cached by changed-file content so repeated agent turns do not repeat work.
"""

from __future__ import annotations

import argparse
import ast
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from typing import Iterable

ROOT = Path(__file__).resolve().parents[1]
GIT_STATE = ROOT / ".git" / "kiro-hooks"
HOOK_DIR = ROOT / ".kiro" / "hooks"
CARGO_JOBS = "2"

EXCLUDED_PREFIXES = (
    ".git/", "target/", "ui/node_modules/", "ui/dist/", "ui/test-results/",
    "ui/playwright-report/", "testing/eval_reports/", "models/",
)

DIRECT_INVOKE_BASELINE = {
    "ui/src/components/AnalyticsDashboard.tsx",
    "ui/src/components/ExportDropdown.tsx",
    "ui/src/components/GuiWorkflowViewer.tsx",
    "ui/src/components/MessageBubble.tsx",
    "ui/src/components/MobileRemotePanel.tsx",
    "ui/src/components/N8nSettings.tsx",
    "ui/src/components/OpenClawSettings.tsx",
    "ui/src/components/ProviderSettings.tsx",
    "ui/src/components/ResourceDashboard.tsx",
    "ui/src/components/SettingsModal.tsx",
    "ui/src/components/SubstrateStatus.tsx",
    "ui/src/components/TestRunnerDashboard.tsx",
    "ui/src/shell/spaces/memory/api/client.ts",
    "ui/src/stores/app.ts",
    "ui/src/stores/memory.ts",
    "ui/src/stores/n8n.ts",
    "ui/src/stores/provisioning.ts",
    "ui/src/views/CapabilitiesView.tsx",
}

# Existing frontend calls not registered in main.rs. Kept as explicit debt so
# hooks block only newly introduced drift; remove entries as contracts are fixed.
UNREGISTERED_COMMAND_BASELINE = {
    "approve_quarantined_tool",
    "check_comfyui_status",
    "get_executive_snapshot",
    "get_policy_gate_log",
    "get_self_model_snapshot",
    "list_quarantined_tools",
    "reject_quarantined_tool",
    "trigger_kill_switch",
}

FILE_EVENTS = {"fileEdited", "fileCreated", "fileDeleted"}
TOOL_EVENTS = {"preToolUse", "postToolUse"}
EVENTS = FILE_EVENTS | TOOL_EVENTS | {
    "userTriggered", "promptSubmit", "agentStop", "preTaskExecution", "postTaskExecution",
}


def git(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args], cwd=ROOT, text=True, capture_output=True, check=check,
    )


def dirty_paths() -> list[str]:
    tracked = git("diff", "--name-only", "--diff-filter=ACMRTUXB", "HEAD", "--").stdout.splitlines()
    untracked = git("ls-files", "--others", "--exclude-standard").stdout.splitlines()
    paths = {path.strip() for path in tracked + untracked if path.strip()}
    return sorted(path for path in paths if not path.startswith(EXCLUDED_PREFIXES))


def content_snapshot(paths: Iterable[str]) -> dict[str, str]:
    snapshot: dict[str, str] = {}
    for relative in paths:
        path = ROOT / relative
        if not path.is_file():
            snapshot[relative] = "<deleted>"
            continue
        digest = hashlib.sha256()
        try:
            digest.update(path.read_bytes())
            snapshot[relative] = digest.hexdigest()
        except OSError:
            snapshot[relative] = "<unreadable>"
    return snapshot


def load_work_state() -> dict[str, object] | None:
    path = GIT_STATE / "work-state.json"
    try:
        data = json.loads(path.read_text())
        if isinstance(data.get("snapshot"), dict) and isinstance(data.get("pending"), list):
            return data
    except (OSError, json.JSONDecodeError):
        pass
    return None


def save_work_state(snapshot: dict[str, str], pending: Iterable[str]) -> None:
    GIT_STATE.mkdir(parents=True, exist_ok=True)
    (GIT_STATE / "work-state.json").write_text(json.dumps({
        "snapshot": snapshot,
        "pending": sorted(set(pending)),
    }, indent=2) + "\n")


def select_changed_files(mode: str) -> tuple[list[str], bool]:
    paths = dirty_paths()
    current = content_snapshot(paths)
    previous = load_work_state()
    if previous is None:
        save_work_state(current, [])
        return [], True

    old_snapshot = previous["snapshot"]
    assert isinstance(old_snapshot, dict)
    delta = {
        path for path in set(old_snapshot) | set(current)
        if old_snapshot.get(path) != current.get(path)
    }
    prior_pending = {str(path) for path in previous["pending"]}
    pending = (prior_pending & set(current)) | (delta & set(current))
    save_work_state(current, pending)
    selected = delta if mode == "quick" else pending
    return sorted(path for path in selected if path in current), False


def clear_pending() -> None:
    state = load_work_state()
    if state is not None:
        snapshot = state["snapshot"]
        assert isinstance(snapshot, dict)
        save_work_state({str(key): str(value) for key, value in snapshot.items()}, [])


def run_command(label: str, command: list[str], cwd: Path = ROOT) -> bool:
    env = os.environ.copy()
    env.setdefault("CARGO_BUILD_JOBS", CARGO_JOBS)
    print(f"[RUN] {label}: {' '.join(command)}")
    result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, env=env)
    if result.returncode == 0:
        print(f"[PASS] {label}")
        return True
    output = (result.stdout + "\n" + result.stderr).strip()
    print(f"[BLOCKING] {label} failed (exit {result.returncode})", file=sys.stderr)
    if output:
        print(output[-12000:], file=sys.stderr)
    return False


def validate_hook_file(path: Path) -> list[str]:
    errors: list[str] = []
    try:
        data = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        return [f"{path.relative_to(ROOT)}: invalid JSON ({error})"]
    for key in ("name", "version", "when", "then"):
        if key not in data:
            errors.append(f"{path.relative_to(ROOT)}: missing '{key}'")
    when = data.get("when", {})
    event = when.get("type")
    if event not in EVENTS:
        errors.append(f"{path.relative_to(ROOT)}: unsupported event '{event}'")
    if event in FILE_EVENTS and not when.get("patterns"):
        errors.append(f"{path.relative_to(ROOT)}: file event requires patterns")
    if event in TOOL_EVENTS and not when.get("toolTypes"):
        errors.append(f"{path.relative_to(ROOT)}: tool event requires toolTypes")
    then = data.get("then", {})
    action = then.get("type")
    if action not in {"askAgent", "runCommand"}:
        errors.append(f"{path.relative_to(ROOT)}: unsupported action '{action}'")
    if action == "askAgent" and not then.get("prompt"):
        errors.append(f"{path.relative_to(ROOT)}: askAgent requires prompt")
    if action == "runCommand" and not then.get("command"):
        errors.append(f"{path.relative_to(ROOT)}: runCommand requires command")
    return errors


def validate_data_files(files: list[str]) -> list[str]:
    errors: list[str] = []
    for relative in files:
        path = ROOT / relative
        if not path.is_file():
            continue
        try:
            if path.suffix == ".json" or path.name.endswith(".kiro.hook"):
                json.loads(path.read_text())
            elif path.suffix == ".toml":
                tomllib.loads(path.read_text())
            elif path.suffix == ".py":
                ast.parse(path.read_text(), filename=str(path))
        except Exception as error:  # reports parser/compiler detail verbatim
            errors.append(f"{relative}: {error}")
    return errors


def validate_conflict_markers(files: list[str]) -> list[str]:
    errors: list[str] = []
    marker = re.compile(r"^(<<<<<<< |=======\s*$|>>>>>>> )", re.MULTILINE)
    for relative in files:
        path = ROOT / relative
        if not path.is_file():
            continue
        if path.stat().st_size > 2_000_000:
            continue
        try:
            text = path.read_text(errors="ignore")
        except OSError:
            continue
        if marker.search(text):
            errors.append(f"{relative}: unresolved merge-conflict marker")
    return errors


def backend_commands() -> set[str]:
    source = (ROOT / "crates/kria-desktop/src/main.rs").read_text()
    match = re.search(r"generate_handler!\s*\[(.*?)\]\s*\)", source, re.DOTALL)
    if not match:
        raise RuntimeError("Tauri generate_handler! registration not found")
    return set(re.findall(r"(?:\w+::)+([A-Za-z_][A-Za-z0-9_]*)\s*,?", match.group(1)))


def frontend_contract() -> tuple[set[str], set[str]]:
    invoked: set[str] = set()
    direct_files: set[str] = set()
    call = re.compile(
        r"\b(?:bridgeInvoke|bridgeInvokeOptional|invoke)(?:<[^;()]*?>)?\s*\(\s*[\"']([A-Za-z0-9_:/.-]+)[\"']"
    )
    for path in (ROOT / "ui/src").rglob("*"):
        relative = path.relative_to(ROOT).as_posix()
        if path.suffix not in {".ts", ".tsx"} or re.search(r"\.(test|spec)\.", path.name):
            continue
        text = path.read_text(errors="ignore")
        invoked.update(call.findall(text))
        if "@tauri-apps/api/core" in text and relative != "ui/src/bridge/invoke.ts":
            direct_files.add(relative)
    return invoked, direct_files


def added_lines(pattern: str) -> list[str]:
    diff = git("diff", "--unified=0", "HEAD", "--", pattern, check=False).stdout
    return [line[1:] for line in diff.splitlines() if line.startswith("+") and not line.startswith("+++")]


def validate_contracts(files: list[str]) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    warnings: list[str] = []
    if not any(path.startswith(("ui/src/", "crates/kria-desktop/src/")) for path in files):
        return errors, warnings
    try:
        invoked, direct = frontend_contract()
        missing = sorted((invoked - backend_commands()) - UNREGISTERED_COMMAND_BASELINE)
        if missing:
            errors.append("frontend invokes unregistered Tauri commands: " + ", ".join(missing))
        new_direct = sorted(direct - DIRECT_INVOKE_BASELINE)
        if new_direct:
            errors.append("new direct Tauri invoke imports bypass ui/src/bridge/invoke.ts: " + ", ".join(new_direct))
    except (OSError, RuntimeError) as error:
        errors.append(f"contract extraction failed: {error}")

    backend_events = set()
    for path in (ROOT / "crates/kria-desktop/src").rglob("*.rs"):
        text = path.read_text(errors="ignore")
        backend_events.update(re.findall(r"\.emit\s*\(\s*&?[\"']([^\"']+)[\"']", text))
    frontend_text = "\n".join(
        path.read_text(errors="ignore") for path in (ROOT / "ui/src").rglob("*.ts*") if path.is_file()
    )
    frontend_events = set(re.findall(r"[\"']([A-Za-z0-9_:/.-]+)[\"']", frontend_text))
    for line in added_lines("crates/kria-desktop/src"):
        match = re.search(r"\.emit\s*\(\s*&?[\"']([^\"']+)[\"']", line)
        if match and match.group(1) not in frontend_events:
            warnings.append(f"new backend event has no literal frontend consumer: {match.group(1)}")
    for line in added_lines("ui/src"):
        match = re.search(r"\blisten(?:<[^>]+>)?\s*\(\s*[\"']([^\"']+)[\"']", line)
        if match and match.group(1) not in backend_events:
            warnings.append(f"new frontend listener has no literal backend producer: {match.group(1)}")
    return errors, warnings


def crate_packages(files: list[str]) -> list[str]:
    crates: set[str] = set()
    for relative in files:
        parts = Path(relative).parts
        if len(parts) >= 2 and parts[0] == "crates":
            manifest = ROOT / "crates" / parts[1] / "Cargo.toml"
            if manifest.exists():
                crates.add(tomllib.loads(manifest.read_text())["package"]["name"])
    return sorted(crates)


def is_security_critical(files: Iterable[str]) -> bool:
    prefixes = (
        "crates/kria-core/src/safety/",
        "crates/kria-core/src/openclaw/",
        "ui/src/shell/approvals/",
    )
    exact = {
        "crates/kria-desktop/src/commands/approval.rs",
        "ui/src/stores/approvalStore.ts",
    }
    return any(path.startswith(prefixes) or path in exact for path in files)


def validate_package_lock(files: list[str]) -> list[str]:
    if "ui/package.json" not in files:
        return []
    package_path = ROOT / "ui/package.json"
    lock_path = ROOT / "ui/package-lock.json"
    if not lock_path.exists():
        return ["ui/package.json changed but ui/package-lock.json is missing"]
    package = json.loads(package_path.read_text())
    lock = json.loads(lock_path.read_text())
    lock_root = lock.get("packages", {}).get("", {})
    errors: list[str] = []
    for section in ("dependencies", "devDependencies", "optionalDependencies"):
        expected = package.get(section, {})
        actual = lock_root.get(section, {})
        if expected != actual:
            errors.append(f"ui/package-lock.json root {section} does not match ui/package.json")
    return errors


def impact_warnings(files: list[str], mode: str) -> list[str]:
    warnings: list[str] = []
    if mode == "final" and len(files) > 100:
        warnings.append(f"large pending change scope ({len(files)} files); review diff boundaries before completion")
    public_change = any(
        path == "crates/kria-desktop/src/main.rs"
        or path.startswith("ui/src/bridge/")
        or path.startswith("config/")
        or path in {".env.example", "Cargo.toml", "ui/package.json", "crates/kria-desktop/tauri.conf.json"}
        for path in files
    )
    docs_changed = any(
        path.startswith("docs/") or path.endswith("README.md") or path.startswith(".kiro/steering/")
        for path in files
    )
    if mode == "final" and public_change and not docs_changed:
        warnings.append("public contract/config/build change has no documentation change; verify docs are intentionally unaffected")
    sensitive = [
        path for path in files
        if Path(path).name == ".env" or Path(path).suffix.lower() in {".pem", ".key", ".p12", ".pfx"}
    ]
    if sensitive:
        warnings.append("sensitive-path changes require explicit secret review: " + ", ".join(sorted(sensitive)))
    return warnings


def plan(files: list[str], mode: str) -> list[str]:
    checks = ["git-diff", "syntax", "conflicts"]
    rust = any(path.endswith(".rs") or path.endswith("Cargo.toml") or path == "Cargo.toml" for path in files)
    ui = any(path.startswith("ui/") for path in files)
    contract = any(path.startswith(("ui/src/", "crates/kria-desktop/src/")) for path in files)
    hooks = any(path.startswith(".kiro/hooks/") for path in files)
    python = any(path.endswith(".py") for path in files)
    if rust:
        checks.append("cargo-fmt")
    if ui:
        checks.append("ui-typecheck")
    if contract:
        checks.append("tauri-contracts")
    if hooks:
        checks.append("hook-schema")
    if python:
        checks.append("python-compile")
    if any(path.startswith(("ui/src/design-system/", "ui/src/kit/", "ui/src/shell/", "ui/src/palette/", "ui/src/features/")) for path in files):
        checks.append("ui-governance")
    if mode == "final":
        if rust:
            checks.extend(["cargo-check", "cargo-test"])
        if ui:
            checks.append("ui-unit")
        if is_security_critical(files):
            checks.append("security-audit")
        if any("n8n" in path.lower() for path in files):
            checks.append("n8n-ci")
        checks.append("secret-scan-if-available")
    return checks


def scan_changed_with_gitleaks(files: list[str]) -> bool:
    executable = shutil.which("gitleaks")
    if not executable:
        print("[INFORMATIONAL] gitleaks unavailable; changed-file secret scan skipped")
        return True
    with tempfile.TemporaryDirectory(prefix="kria-gitleaks-") as directory:
        staging = Path(directory)
        for relative in files:
            source = ROOT / relative
            if not source.is_file():
                continue
            destination = staging / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        return run_command(
            "changed-file secret scan",
            [executable, "dir", "--no-banner", "--redact", "--no-color", str(staging)],
        )


def execute(mode: str, files: list[str]) -> bool:
    ok = True
    print(f"[INFORMATIONAL] mode={mode}; changed_files={len(files)}; checks={','.join(plan(files, mode))}")
    if not run_command("Git whitespace/error check", ["git", "diff", "--check", "HEAD", "--", *files]):
        ok = False

    errors = validate_data_files(files) + validate_conflict_markers(files) + validate_package_lock(files)
    if any(path.startswith(".kiro/hooks/") for path in files):
        for path in sorted(HOOK_DIR.glob("*.kiro.hook")):
            errors.extend(validate_hook_file(path))
    contract_errors, warnings = validate_contracts(files)
    errors.extend(contract_errors)
    warnings.extend(impact_warnings(files, mode))
    for warning in warnings:
        print(f"[WARNING] {warning}")
    for error in errors:
        print(f"[BLOCKING] {error}", file=sys.stderr)
    ok = ok and not errors

    rust_files = [str(ROOT / path) for path in files if path.endswith(".rs") and (ROOT / path).is_file()]
    rust = bool(rust_files) or any(path.endswith("Cargo.toml") or path == "Cargo.toml" for path in files)
    ui = any(path.startswith("ui/") for path in files)
    if rust_files:
        ok = run_command("Changed Rust format", ["rustfmt", "--edition", "2021", "--check", *rust_files]) and ok
    if ui:
        ok = run_command("UI typecheck", ["npm", "run", "check"], ROOT / "ui") and ok
    if any(path.startswith(("ui/src/design-system/", "ui/src/kit/", "ui/src/shell/", "ui/src/palette/", "ui/src/features/")) for path in files):
        ok = run_command("UI architecture consistency", ["npm", "run", "lint:ui-consistency"], ROOT / "ui") and ok

    if mode == "final":
        packages = crate_packages(files)
        if rust and not packages:
            ok = run_command("Workspace Cargo check", ["cargo", "check", "--workspace"]) and ok
        for package in packages:
            ok = run_command(f"Cargo check {package}", ["cargo", "check", "-p", package]) and ok
            ok = run_command(f"Cargo test {package}", ["cargo", "test", "-p", package]) and ok
        if ui:
            ok = run_command("UI unit tests", ["npm", "run", "test:run", "--", "--maxWorkers=1", "--minWorkers=1"], ROOT / "ui") and ok
        if is_security_critical(files):
            ok = run_command("Security audit suite", ["./testing/run.sh", "security_audit", "--profile", "ci", "--fail-fast"]) and ok
        if any("n8n" in path.lower() for path in files):
            ok = run_command("n8n CI-safe suite", ["./testing/run.sh", "n8n", "--profile", "ci", "--fail-fast"]) and ok
        ok = scan_changed_with_gitleaks(files) and ok
    return ok


def validate_scenarios() -> bool:
    scenarios = {
        "frontend component": (["ui/src/components/Button.tsx"], {"ui-typecheck", "tauri-contracts"}),
        "Rust core": (["crates/kria-core/src/agent/mod.rs"], {"cargo-fmt", "cargo-check"}),
        "Tauri command": (["crates/kria-desktop/src/commands/chat.rs"], {"tauri-contracts", "cargo-test"}),
        "frontend bridge": (["ui/src/bridge/invoke.ts"], {"tauri-contracts", "ui-unit"}),
        "OpenClaw safety": (["crates/kria-core/src/openclaw/mod.rs"], {"security-audit", "cargo-test"}),
        "memory persistence": (["crates/kria-core/src/memory/store.rs"], {"cargo-test"}),
        "Cargo manifest": (["Cargo.toml"], {"cargo-check"}),
        "package manifest": (["ui/package.json"], {"ui-typecheck", "ui-unit"}),
        "documentation": (["docs/ARCHITECTURE.md"], {"git-diff"}),
        "cross-cutting": (["Cargo.toml", "crates/kria-desktop/src/main.rs", "ui/src/bridge/invoke.ts"], {"cargo-check", "ui-unit", "tauri-contracts"}),
    }
    ok = True
    for name, (files, expected) in scenarios.items():
        actual = set(plan(files, "final"))
        missing = sorted(expected - actual)
        if missing:
            ok = False
            print(f"[BLOCKING] scenario '{name}' missing checks: {', '.join(missing)}", file=sys.stderr)
        else:
            print(f"[PASS] scenario '{name}': {','.join(sorted(actual))}")
    return ok


def self_test() -> bool:
    ok = validate_scenarios()
    hook_errors = [
        error
        for path in sorted(HOOK_DIR.glob("*.kiro.hook"))
        for error in validate_hook_file(path)
    ]
    contract_errors, contract_warnings = validate_contracts([
        "crates/kria-desktop/src/main.rs",
        "ui/src/bridge/invoke.ts",
    ])
    lock_errors = validate_package_lock(["ui/package.json"])
    for warning in contract_warnings:
        print(f"[WARNING] {warning}")
    for error in hook_errors + contract_errors + lock_errors:
        print(f"[BLOCKING] {error}", file=sys.stderr)
    errors = hook_errors + contract_errors + lock_errors
    if not errors:
        print(f"[PASS] self-test: {len(list(HOOK_DIR.glob('*.kiro.hook')))} hooks, contracts, lockfile, scenarios")
    return ok and not errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "mode",
        choices=("quick", "final", "init-baseline", "validate-hooks", "scenario-matrix", "self-test"),
    )
    args = parser.parse_args()

    if args.mode == "validate-hooks":
        errors = [error for path in sorted(HOOK_DIR.glob("*.kiro.hook")) for error in validate_hook_file(path)]
        for error in errors:
            print(f"[BLOCKING] {error}", file=sys.stderr)
        print(f"[{'PASS' if not errors else 'BLOCKING'}] validated {len(list(HOOK_DIR.glob('*.kiro.hook')))} hook files")
        return 0 if not errors else 1
    if args.mode == "scenario-matrix":
        return 0 if validate_scenarios() else 1
    if args.mode == "self-test":
        return 0 if self_test() else 1
    if args.mode == "init-baseline":
        paths = dirty_paths()
        save_work_state(content_snapshot(paths), [])
        print(f"[PASS] initialized hook baseline with {len(paths)} pre-existing dirty paths")
        return 0

    GIT_STATE.mkdir(parents=True, exist_ok=True)
    lock_path = GIT_STATE / "validation.lock"
    with lock_path.open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("[INFORMATIONAL] validation already running; duplicate hook skipped")
            return 0
        files, initialized = select_changed_files(args.mode)
        if initialized:
            print("[INFORMATIONAL] existing dirty work baselined; no historical files validated")
            return 0
        if not files:
            scope = "new changes" if args.mode == "quick" else "pending task changes"
            print(f"[INFORMATIONAL] no {scope}; validation skipped")
            return 0
        success = execute(args.mode, files)
        if success and args.mode == "final":
            clear_pending()
        return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
