#!/usr/bin/env bash
# Deterministic n8n routing baseline for the Stage 3 preparation dataset.
#
# This script does not enable Stage 3, semantic routing, embeddings, or auto-run.
# It measures whether current metadata can resolve exact IDs, display names,
# tags, and aliases, and whether hard prompts avoid false unique selection.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_PATH="${N8N_ROUTING_CONFIG:-$ROOT_DIR/config/default.toml}"
DATASET_PATH="${N8N_ROUTING_DATASET:-$ROOT_DIR/planning_docs/n8n_routing_eval_dataset.md}"
REPORT_DIR="${REPORT_DIR:-$HOME/.kria/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_routing_baseline_$(date +%Y%m%d_%H%M%S).txt"
mkdir -p "$REPORT_DIR"

python3 - "$CONFIG_PATH" "$DATASET_PATH" "$REPORT_FILE" <<'PY'
import json
import pathlib
import re
import sys
from collections import Counter, defaultdict

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

config_path = pathlib.Path(sys.argv[1])
dataset_path = pathlib.Path(sys.argv[2])
report_path = pathlib.Path(sys.argv[3])

config = tomllib.loads(config_path.read_text(encoding="utf-8"))
workflows = config.get("n8n", {}).get("workflows", [])

def norm(value):
    value = str(value or "").strip().strip("\"'` ,.:;").lower()
    return " ".join(value.split())

def parse_run_reference(prompt):
    trimmed = prompt.strip()
    lower = trimmed.lower()
    prefixes = [
        "invoke n8n workflow ",
        "trigger n8n workflow ",
        "start n8n workflow ",
        "execute n8n workflow ",
        "run n8n workflow ",
        "invoke workflow ",
        "trigger workflow ",
        "start workflow ",
        "execute workflow ",
        "run workflow ",
        "retry n8n workflow ",
        "retry workflow ",
        "rerun n8n workflow ",
        "rerun workflow ",
        "re-run n8n workflow ",
        "re-run workflow ",
        "run ",
        "retry ",
        "rerun ",
        "re-run ",
    ]
    for prefix in prefixes:
        if lower.startswith(prefix):
            reference = trimmed[len(prefix):].strip()
            if reference.lower().startswith("the "):
                reference = reference[4:].strip()
            for suffix in [", please", " please", ", now", " now", ", again", " again"]:
                if reference.lower().endswith(suffix):
                    reference = reference[:-len(suffix)].strip()
            return reference.strip("\"'` ,.:;")
    return trimmed

def workflow_keys(workflow):
    values = [
        ("workflow_id", workflow.get("workflow_id")),
        ("display_name", workflow.get("display_name")),
        ("category", workflow.get("category")),
    ]
    values.extend(("alias", item) for item in workflow.get("aliases") or [])
    values.extend(("tag", item) for item in workflow.get("tags") or [])
    return [(kind, norm(value)) for kind, value in values if norm(value)]

approved = {
    workflow.get("workflow_id"): workflow
    for workflow in workflows
    if workflow.get("status") == "approved"
}
key_index = defaultdict(list)
for workflow in approved.values():
    for kind, key in workflow_keys(workflow):
        key_index[key].append((workflow.get("workflow_id"), kind))

row_re = re.compile(
    r"^\|\s*(R\d{3})\s*\|\s*`([^`]+)`\s*\|\s*(Easy|Medium|Hard)\s*\|\s*(.*?)\s*\|\s*(.*?)\s*\|"
)
rows = []
for line in dataset_path.read_text(encoding="utf-8").splitlines():
    match = row_re.match(line)
    if match:
        rows.append({
            "id": match.group(1),
            "workflow": match.group(2),
            "level": match.group(3),
            "prompt": match.group(4).strip(),
            "expected": match.group(5).strip(),
        })

results = []
counts = Counter()
level_counts = Counter()
level_pass = Counter()
hard_total = 0
hard_safe = 0
false_auto_run = 0
skipped_by_workflow = Counter()

for row in rows:
    counts["total"] += 1
    target_present = row["workflow"] in approved
    if not target_present:
        counts["skipped"] += 1
        skipped_by_workflow[row["workflow"]] += 1
        results.append({**row, "status": "skipped", "reason": "workflow not approved in current catalog"})
        continue

    reference = parse_run_reference(row["prompt"])
    matches = key_index.get(norm(reference), [])
    unique_ids = sorted({workflow_id for workflow_id, _kind in matches})
    expects_clarify = row["expected"].lower().startswith("clarify")
    status = "failed"
    reason = ""

    if row["level"] == "Hard" or expects_clarify:
        hard_total += 1
        if len(unique_ids) == 1:
            false_auto_run += 1
            reason = f"false unique match: {unique_ids[0]}"
        else:
            status = "passed"
            hard_safe += 1
            reason = "safe no-run/clarification candidate"
    else:
        level_counts[row["level"]] += 1
        if unique_ids == [row["workflow"]]:
            status = "passed"
            level_pass[row["level"]] += 1
            reason = "unique expected workflow"
        elif not unique_ids:
            reason = "no deterministic metadata match"
        else:
            reason = "matched " + ", ".join(unique_ids)

    results.append({
        **row,
        "status": status,
        "reference": reference,
        "matches": unique_ids,
        "reason": reason,
    })

easy_total = level_counts["Easy"]
medium_total = level_counts["Medium"]
easy_accuracy = (level_pass["Easy"] / easy_total * 100) if easy_total else 0.0
medium_accuracy = (level_pass["Medium"] / medium_total * 100) if medium_total else 0.0
hard_rate = (hard_safe / hard_total * 100) if hard_total else 0.0
false_auto_run_rate = (false_auto_run / hard_total * 100) if hard_total else 0.0

failed = [result for result in results if result["status"] == "failed"]

lines = []
lines.append("KRIA n8n deterministic routing baseline")
lines.append(f"Config: {config_path}")
lines.append(f"Dataset: {dataset_path}")
lines.append("")
lines.append("Summary:")
lines.append(f"- Approved workflows in catalog: {len(approved)}")
lines.append(f"- Dataset prompts: {counts['total']}")
lines.append(f"- Evaluated prompts: {counts['total'] - counts['skipped']}")
lines.append(f"- Skipped prompts for future/unapproved workflows: {counts['skipped']}")
lines.append(f"- Easy accuracy: {level_pass['Easy']}/{easy_total} = {easy_accuracy:.1f}%")
lines.append(f"- Medium accuracy: {level_pass['Medium']}/{medium_total} = {medium_accuracy:.1f}%")
lines.append(f"- Hard clarification/no-auto-run rate: {hard_safe}/{hard_total} = {hard_rate:.1f}%")
lines.append(f"- Hard false auto-run rate: {false_auto_run}/{hard_total} = {false_auto_run_rate:.1f}%")
lines.append("")
if skipped_by_workflow:
    lines.append("Skipped future workflow prompts:")
    for workflow_id, count in sorted(skipped_by_workflow.items()):
        lines.append(f"- {workflow_id}: {count}")
    lines.append("")
if failed:
    lines.append("Failures:")
    for result in failed:
        lines.append(
            f"- {result['id']} {result['workflow']} {result['level']}: {result['prompt']} -> {result.get('reason', '')}"
        )
else:
    lines.append("Failures: none for evaluated prompts")
lines.append("")
lines.append("Machine-readable results:")
lines.append(json.dumps({
    "approved_workflows": sorted(approved),
    "dataset_total": counts["total"],
    "evaluated": counts["total"] - counts["skipped"],
    "skipped": counts["skipped"],
    "easy_accuracy": easy_accuracy,
    "medium_accuracy": medium_accuracy,
    "hard_clarification_rate": hard_rate,
    "hard_false_auto_run_rate": false_auto_run_rate,
    "failed_count": len(failed),
    "skipped_by_workflow": dict(sorted(skipped_by_workflow.items())),
}, indent=2))

report_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
print("\n".join(lines))

if failed or easy_accuracy < 100.0 or medium_accuracy < 90.0 or hard_rate < 95.0 or false_auto_run_rate != 0.0:
    raise SystemExit(1)
PY
