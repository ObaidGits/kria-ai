#!/usr/bin/env python3
"""
Zero-touch updater for KRIA's AI context docs.

Default behavior is optimized for post-commit use:
- read config from ai-context/config.json
- skip when disabled
- skip when the current commit was already processed
- inspect only the last commit with `git diff --name-only HEAD~1 HEAD`
- update only generated "change watch" sections in affected docs

Manual prose outside generated blocks is never rewritten.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


DOCS = {
    "CODEBASE_MAP.md": "structural changes",
    "FILE_INDEX.md": "important file-level changes",
    "COMMON_FLOWS.md": "logic or control-flow changes",
}

GENERATED_HEADERS = {
    "CODEBASE_MAP.md": "## Structure Change Watch",
    "FILE_INDEX.md": "## File Index Change Watch",
    "COMMON_FLOWS.md": "## Flow Change Watch",
}

DEFAULT_CONFIG: dict[str, Any] = {
    "enable_auto_update": True,
    "fast_mode": True,
    "watched_paths": [
        "crates/kria-core/src",
        "crates/kria-desktop/src",
        "crates/kria-server/src",
        "kria-modules/src",
        "ui/src",
        "Cargo.toml",
    ],
    "ignore_paths": [
        "target",
        "ui/node_modules",
        "ui/dist",
        ".pytest_cache",
        ".ruff_cache",
        "__pycache__",
        ".git",
        "ai-context",
        "models",
        "tests/e2e/test-results",
        "tests/e2e/playwright-report",
    ],
}

SOURCE_EXTS = {".rs", ".py", ".ts", ".tsx", ".toml", ".json", ".yml", ".yaml"}
DOC_ONLY_NAMES = {"README.md", "LICENSE"}
IMPORTANT_FILENAMES = {
    "Cargo.toml",
    "Cargo.lock",
    "package.json",
    "vite.config.ts",
    "tauri.conf.json",
    "config.rs",
    "lib.rs",
    "main.rs",
    "mod.rs",
    "commands.rs",
    "loop_engine.rs",
    "runtime.rs",
    "chat.rs",
    "sessions.rs",
    "voice.rs",
    "local_api.rs",
    "registry.rs",
    "policy.rs",
    "bridge.py",
    "app.ts",
    "App.tsx",
}

STRUCTURAL_HINTS = (
    "Cargo.toml",
    "crates/kria-core/src/lib.rs",
    "crates/kria-core/src/config.rs",
    "crates/kria-desktop/src/main.rs",
    "crates/kria-server/src/main.rs",
    "crates/kria-server/src/lib.rs",
    "kria-modules/src/kria_modules/bridge.py",
    "ui/src/App.tsx",
)

FLOW_HINTS = (
    "crates/kria-desktop/src/commands.rs",
    "crates/kria-desktop/src/commands/",
    "crates/kria-core/src/agent/",
    "crates/kria-core/src/tools/",
    "crates/kria-core/src/safety/",
    "crates/kria-core/src/routing/",
    "crates/kria-core/src/sidecar/",
    "crates/kria-core/src/mcp/",
    "crates/kria-core/src/voice/",
    "crates/kria-core/src/image/",
    "crates/kria-server/src/routes.rs",
    "crates/kria-server/src/ws.rs",
    "ui/src/stores/app.ts",
    "kria-modules/src/kria_modules/bridge.py",
)

COMMENT_PREFIXES = ("//", "#", "*", "/*", "*/")


@dataclass(frozen=True)
class Change:
    status: str
    path: str
    old_path: str | None = None

    @property
    def is_add(self) -> bool:
        return self.status.startswith("A") or self.status.startswith("R")

    @property
    def is_delete(self) -> bool:
        return self.status.startswith("D")

    @property
    def is_modify(self) -> bool:
        return self.status.startswith("M")

    @property
    def is_rename(self) -> bool:
        return self.status.startswith("R")


def run_git(args: list[str], repo: Path) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=repo,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "git command failed")
    return result.stdout


def repo_root() -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise SystemExit(0)
    return Path(result.stdout.strip())


def load_json(path: Path, fallback: dict[str, Any]) -> dict[str, Any]:
    if not path.exists():
        return dict(fallback)
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return dict(fallback)
    merged = dict(fallback)
    merged.update(data if isinstance(data, dict) else {})
    return merged


def save_json(path: Path, data: dict[str, Any]) -> None:
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def current_commit(repo: Path) -> str:
    return run_git(["rev-parse", "HEAD"], repo).strip()


def path_matches(path: str, prefixes: list[str] | tuple[str, ...]) -> bool:
    clean = path.strip("/")
    return any(clean == p.strip("/") or clean.startswith(p.strip("/") + "/") for p in prefixes)


def ignored(path: str, config: dict[str, Any]) -> bool:
    if path.endswith((".pyc", ".pyo")) or "/__pycache__/" in f"/{path}":
        return True
    return path_matches(path, tuple(config.get("ignore_paths", [])))


def watched(path: str, config: dict[str, Any]) -> bool:
    watched_paths = tuple(config.get("watched_paths", []))
    return not watched_paths or path_matches(path, watched_paths)


def parse_name_status(raw: str) -> dict[str, Change]:
    changes: dict[str, Change] = {}
    for line in raw.splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        status = parts[0]
        if status.startswith("R") and len(parts) >= 3:
            changes[parts[2]] = Change(status=status, old_path=parts[1], path=parts[2])
        elif len(parts) >= 2:
            changes[parts[1]] = Change(status=status, path=parts[1])
    return changes


def last_commit_changes(repo: Path, config: dict[str, Any], fast_mode: bool) -> list[Change]:
    # Required detector: cheap name-only scan for the last commit.
    names_raw = run_git(["diff", "--name-only", "HEAD~1", "HEAD"], repo)
    names = [line.strip() for line in names_raw.splitlines() if line.strip()]

    status_raw = run_git(["diff", "--name-status", "--find-renames", "HEAD~1", "HEAD"], repo)
    status_by_path = parse_name_status(status_raw)

    changes: list[Change] = []
    for path in names:
        if ignored(path, config):
            continue
        if fast_mode and not watched(path, config):
            continue
        changes.append(status_by_path.get(path, Change(status="M", path=path)))
    return changes


def is_source(path: str) -> bool:
    return Path(path).suffix in SOURCE_EXTS


def is_doc_only(path: str) -> bool:
    p = Path(path)
    return p.name in DOC_ONLY_NAMES or (path.startswith("docs/") and "ARCHITECTURE" not in path.upper())


def is_important_file(path: str) -> bool:
    p = Path(path)
    return p.name in IMPORTANT_FILENAMES or path_matches(path, STRUCTURAL_HINTS)


def has_substantive_diff(repo: Path, path: str) -> bool:
    if not is_source(path):
        return False
    try:
        raw = run_git(["diff", "--unified=0", "HEAD~1", "HEAD", "--", path], repo)
    except Exception:
        return True

    for line in raw.splitlines():
        if not line.startswith(("+", "-")) or line.startswith(("+++", "---")):
            continue
        text = line[1:].strip()
        if not text:
            continue
        if text.startswith(COMMENT_PREFIXES):
            continue
        return True
    return False


def classify_docs(repo: Path, changes: list[Change]) -> dict[str, list[str]]:
    affected: dict[str, list[str]] = {name: [] for name in DOCS}

    for change in changes:
        paths = [p for p in (change.old_path, change.path) if p]
        if all(is_doc_only(p) for p in paths):
            continue

        structural = any(path_matches(p, STRUCTURAL_HINTS) for p in paths)
        file_level = (change.is_add or change.is_delete or change.is_rename) and any(
            is_important_file(p) for p in paths
        )
        flow = any(path_matches(p, FLOW_HINTS) for p in paths)

        if change.is_modify and not structural and not flow and not any(is_important_file(p) for p in paths):
            continue

        if change.is_modify and is_source(change.path) and not has_substantive_diff(repo, change.path):
            continue

        if structural or change.is_add or change.is_delete or change.is_rename:
            if any(is_source(p) for p in paths) or structural:
                affected["CODEBASE_MAP.md"].append(change.path)

        if file_level:
            affected["FILE_INDEX.md"].append(change.path)

        if flow:
            affected["COMMON_FLOWS.md"].append(change.path)

    return {doc: sorted(set(paths)) for doc, paths in affected.items() if paths}


def generated_block(doc_name: str, changes: list[Change], paths: list[str], commit: str) -> str:
    now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    header = GENERATED_HEADERS[doc_name]
    selected = [c for c in changes if c.path in paths or (c.old_path and c.old_path in paths)]

    lines = [
        header,
        "<!-- AI-CONTEXT:START generated-change-watch -->",
        f"- Last checked: {now}",
        f"- Commit: `{commit[:12]}`",
        f"- Reason: {DOCS[doc_name]}.",
    ]

    for change in selected[:12]:
        if change.is_rename and change.old_path:
            lines.append(f"- `{change.old_path}` -> `{change.path}` renamed; verify stable docs mention the new path.")
        elif change.is_add:
            lines.append(f"- `{change.path}` added; include only if it becomes an important navigation point.")
        elif change.is_delete:
            lines.append(f"- `{change.path}` removed; remove stale references if present.")
        elif doc_name == "COMMON_FLOWS.md":
            lines.append(f"- `{change.path}` changed flow-adjacent logic; review exact behavior (needs verification).")
        else:
            lines.append(f"- `{change.path}` changed; review stable summary only if its public role changed.")

    if len(selected) > 12:
        lines.append(f"- {len(selected) - 12} more relevant files omitted for brevity.")

    prompt_hint = {
        "CODEBASE_MAP.md": "Use `ai-context/prompts/update_map.txt` only if stable structure changed.",
        "FILE_INDEX.md": "Use `ai-context/prompts/update_index.txt` only for important file additions/removals.",
        "COMMON_FLOWS.md": "Use `ai-context/prompts/update_flows.txt` only when behavior changed.",
    }[doc_name]
    lines.extend(
        [
            f"- Next action: {prompt_hint}",
            "<!-- AI-CONTEXT:END generated-change-watch -->",
            "",
        ]
    )
    return "\n".join(lines)


def replace_generated_section(text: str, header: str, block: str) -> str:
    start_marker = "<!-- AI-CONTEXT:START generated-change-watch -->"
    end_marker = "<!-- AI-CONTEXT:END generated-change-watch -->"
    start = text.find(start_marker)
    end = text.find(end_marker)

    if start != -1 and end != -1 and start < end:
        heading_start = text.rfind("\n## ", 0, start)
        replace_start = heading_start + 1 if heading_start != -1 else start
        replace_end = end + len(end_marker)
        while replace_end < len(text) and text[replace_end] in "\n":
            replace_end += 1
        return text[:replace_start] + block + text[replace_end:]

    if header in text:
        idx = text.find(header)
        next_heading = text.find("\n## ", idx + len(header))
        if next_heading == -1:
            return text[:idx].rstrip() + "\n\n" + block
        return text[:idx].rstrip() + "\n\n" + block + text[next_heading:]

    return text.rstrip() + "\n\n" + block


def update_doc(repo: Path, doc_name: str, changes: list[Change], paths: list[str], commit: str, dry_run: bool) -> str:
    doc_path = repo / "ai-context" / doc_name
    if not doc_path.exists():
        return f"{doc_name}: missing, skipped"

    original = doc_path.read_text(encoding="utf-8")
    updated = replace_generated_section(
        original,
        GENERATED_HEADERS[doc_name],
        generated_block(doc_name, changes, paths, commit),
    )
    if updated == original:
        return f"{doc_name}: unchanged"
    if not dry_run:
        doc_path.write_text(updated, encoding="utf-8")
    return f"{doc_name}: updated generated watch"


def print_changes(changes: list[Change]) -> None:
    print("Changed files:")
    if not changes:
        print("  none")
        return
    for change in changes:
        if change.is_rename and change.old_path:
            print(f"  {change.status} {change.old_path} -> {change.path}")
        else:
            print(f"  {change.status} {change.path}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Zero-touch KRIA AI context updater.")
    parser.add_argument("--dry-run", action="store_true", help="Show planned updates without writing")
    parser.add_argument("--force", action="store_true", help="Process current commit even if cached")
    parser.add_argument("--no-cache", action="store_true", help="Do not read/write processed commit cache")
    parser.add_argument("--full", action="store_true", help="Disable fast-mode path filtering for this run")
    args = parser.parse_args()

    repo = repo_root()
    context_dir = repo / "ai-context"
    config = load_json(context_dir / "config.json", DEFAULT_CONFIG)

    if not config.get("enable_auto_update", True):
        print("ai-context auto-update disabled by config.")
        return 0

    commit = current_commit(repo)
    cache_path = context_dir / ".cache.json"
    cache = load_json(cache_path, {}) if not args.no_cache else {}
    if not args.force and not args.no_cache and cache.get("last_processed_commit") == commit:
        print(f"ai-context already processed commit {commit[:12]}; skipping.")
        return 0

    fast_mode = bool(config.get("fast_mode", True)) and not args.full

    try:
        changes = last_commit_changes(repo, config, fast_mode)
        affected = classify_docs(repo, changes)
    except Exception as exc:
        print(f"ai-context update skipped after error: {exc}", file=sys.stderr)
        return 0

    print(f"Commit: {commit[:12]}")
    print(f"Fast mode: {'on' if fast_mode else 'off'}")
    print_changes(changes)

    if not affected:
        print("No relevant AI-context updates.")
        if not args.dry_run and not args.no_cache:
            save_json(cache_path, {"last_processed_commit": commit, "last_checked_utc": dt.datetime.now(dt.timezone.utc).isoformat()})
        return 0

    print("\nAffected docs:")
    for doc, paths in affected.items():
        print(f"  {doc}: {len(paths)} relevant change(s)")

    print("\nUpdates:")
    for doc, paths in affected.items():
        print(f"  {update_doc(repo, doc, changes, paths, commit, args.dry_run)}")

    if not args.dry_run and not args.no_cache:
        save_json(
            cache_path,
            {
                "last_processed_commit": commit,
                "last_checked_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
                "updated_docs": sorted(affected.keys()),
            },
        )

    print("\nSummary: updated only generated change-watch sections; manual prose preserved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
