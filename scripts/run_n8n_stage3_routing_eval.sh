#!/usr/bin/env bash
# KRIA n8n Stage 3 bounded routing eval.
#
# This evaluates metadata-only workflow suggestions. It must not use LLM
# scoring, embeddings, semantic search, vector DBs, or automatic execution.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REGISTRY_PATH="${N8N_WORKFLOW_REGISTRY:-$HOME/.kria/n8n/workflow_registry.json}"
CONFIG_PATH="${N8N_ROUTING_CONFIG:-$ROOT_DIR/config/default.toml}"
DATASET_PATH="${N8N_ROUTING_DATASET:-$ROOT_DIR/planning_docs/n8n_routing_eval_dataset.md}"
REPORT_DIR="${REPORT_DIR:-$HOME/.kria/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_stage3_routing_eval_$(date +%Y%m%d_%H%M%S).txt"
mkdir -p "$REPORT_DIR"

python3 - "$REGISTRY_PATH" "$CONFIG_PATH" "$DATASET_PATH" "$REPORT_FILE" <<'PY'
import json
import pathlib
import re
import sys
from collections import Counter, defaultdict

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

registry_path = pathlib.Path(sys.argv[1])
config_path = pathlib.Path(sys.argv[2])
dataset_path = pathlib.Path(sys.argv[3])
report_path = pathlib.Path(sys.argv[4])

source = f"workflow registry: {registry_path}"
workflows = []
if registry_path.exists():
    registry = json.loads(registry_path.read_text(encoding="utf-8"))
    for record in registry.get("workflows", []):
        workflow = record.get("workflow") if isinstance(record.get("workflow"), dict) else record
        if isinstance(workflow, dict):
            workflows.append(workflow)
else:
    source = f"legacy TOML fallback: {config_path}"
    config = tomllib.loads(config_path.read_text(encoding="utf-8"))
    workflows = config.get("n8n", {}).get("workflows", [])

STOPWORDS = {
    "a", "an", "and", "are", "at", "by", "for", "from", "i", "in", "is",
    "it", "me", "my", "of", "on", "or", "please", "the", "this", "to",
    "with",
}
HARD_PHRASES = {
    "brief me",
    "check if automation is healthy",
    "clean up",
    "discuss the report",
    "find out what everyone sent",
    "get the report from mail",
    "handle my email",
    "handle the client follow-up",
    "handle this payment thing",
    "organize bug reports",
    "process this document",
    "publish the update",
    "reply to everyone",
    "send the report to the team",
    "share the report with everyone",
    "summarize everything",
    "test everything",
    "track this bug",
}

def norm(value):
    value = str(value or "").strip().strip("\"'` ,.:;").lower()
    return " ".join(value.split())

def normalize_tokens(value):
    text = "".join(ch.lower() if ch.isalnum() or ch == "_" else " " for ch in str(value or ""))
    return " ".join(text.split())

def tokens(value):
    return {
        token for token in normalize_tokens(value).split()
        if len(token) > 1 and token not in STOPWORDS
    }

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

def metadata_keys(workflow):
    keys = [
        ("workflow_id", workflow.get("workflow_id"), 100, 86),
        ("display_name", workflow.get("display_name"), 96, 82),
        ("category", workflow.get("category"), 62, 48),
    ]
    keys.extend(("alias", item, 92, 78) for item in workflow.get("aliases") or [])
    keys.extend(("tag", item, 74, 58) for item in workflow.get("tags") or [])
    keys.extend(("example_prompt", item, 88, 76) for item in workflow.get("example_prompts") or [])
    return [(kind, norm(value), exact, contains) for kind, value, exact, contains in keys if norm(value)]

approved = [
    workflow for workflow in workflows
    if workflow.get("status") == "approved"
]

def rank(prompt):
    reference = parse_run_reference(prompt)
    normalized_reference = norm(reference)
    reference_tokens = tokens(reference)
    candidates = []
    for workflow in approved:
        best_score = 0.0
        matched_on = set()
        reason = ""
        for kind, key, exact_score, contains_score in metadata_keys(workflow):
            if key == normalized_reference:
                if exact_score > best_score:
                    best_score = exact_score
                    reason = f"Exact {kind} match"
                matched_on.add(kind)
                continue
            if len(key) >= 4 and len(normalized_reference) >= 4 and (key in normalized_reference or normalized_reference in key):
                if contains_score > best_score:
                    best_score = contains_score
                    reason = f"Phrase overlap with {kind}"
                matched_on.add(kind)
                continue
            key_tokens = tokens(key)
            if not key_tokens or not reference_tokens:
                continue
            overlap = len(key_tokens & reference_tokens)
            ratio = overlap / max(len(key_tokens), len(reference_tokens))
            if overlap >= 2 and ratio >= 0.45:
                score = max(contains_score * ratio, 44.0)
                if score > best_score:
                    best_score = score
                    reason = f"Token overlap with {kind}"
                matched_on.add(kind)
        if best_score >= 44.0:
            candidates.append({
                "workflow_id": workflow.get("workflow_id"),
                "score": round(best_score, 1),
                "matched_on": sorted(matched_on),
                "reason": reason,
            })
    candidates.sort(key=lambda item: (-item["score"], item["workflow_id"]))
    candidates = candidates[:3]
    hard_prompt = norm(reference) in HARD_PHRASES or any(phrase in norm(reference) for phrase in HARD_PHRASES)
    ambiguous = (
        len(candidates) > 1
        and any(candidates[0]["score"] - item["score"] <= 18.0 for item in candidates[1:])
    )
    if candidates:
        status = "needs_clarification" if hard_prompt or ambiguous else "needs_confirmation"
    elif hard_prompt:
        status = "needs_clarification"
    else:
        status = "not_found"
    return {
        "reference": reference,
        "status": status,
        "candidates": candidates,
        "hard_prompt": hard_prompt,
        "ambiguous": ambiguous,
        "requires_confirmation": True,
        "can_auto_run": False,
    }

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

approved_ids = {workflow.get("workflow_id") for workflow in approved}
counts = Counter()
level_total = Counter()
level_pass = Counter()
skipped_by_workflow = Counter()
failures = []
false_auto_run = 0
hard_total = 0
hard_safe = 0

for row in rows:
    counts["total"] += 1
    if row["workflow"] not in approved_ids:
        counts["skipped"] += 1
        skipped_by_workflow[row["workflow"]] += 1
        continue

    result = rank(row["prompt"])
    top_id = result["candidates"][0]["workflow_id"] if result["candidates"] else None
    expects_clarification = row["level"] == "Hard" or row["expected"].lower().startswith("clarify")
    if result["can_auto_run"]:
        false_auto_run += 1

    if expects_clarification:
        hard_total += 1
        if not result["can_auto_run"] and result["status"] == "needs_clarification":
            hard_safe += 1
        else:
            failures.append((row, result, "hard prompt did not force clarification"))
        continue

    level_total[row["level"]] += 1
    if top_id == row["workflow"] and result["status"] == "needs_confirmation" and not result["can_auto_run"]:
        level_pass[row["level"]] += 1
    else:
        failures.append((row, result, f"expected top candidate {row['workflow']}, got {top_id}"))

easy_total = level_total["Easy"]
medium_total = level_total["Medium"]
easy_accuracy = (level_pass["Easy"] / easy_total * 100.0) if easy_total else 0.0
medium_accuracy = (level_pass["Medium"] / medium_total * 100.0) if medium_total else 0.0
hard_rate = (hard_safe / hard_total * 100.0) if hard_total else 0.0
false_auto_run_rate = (false_auto_run / max(counts["total"] - counts["skipped"], 1) * 100.0)
evaluated = counts["total"] - counts["skipped"]

lines = [
    "KRIA n8n Stage 3 bounded routing eval",
    f"Workflow source: {source}",
    f"Dataset: {dataset_path}",
    "",
    "Summary:",
    f"- Approved workflows in catalog: {len(approved)}",
    f"- Dataset prompts: {counts['total']}",
    f"- Evaluated prompts: {evaluated}",
    f"- Skipped future/unapproved prompts: {counts['skipped']}",
    f"- Easy accuracy: {level_pass['Easy']}/{easy_total} = {easy_accuracy:.1f}%",
    f"- Medium accuracy: {level_pass['Medium']}/{medium_total} = {medium_accuracy:.1f}%",
    f"- Hard clarification rate: {hard_safe}/{hard_total} = {hard_rate:.1f}%",
    f"- False auto-run rate: {false_auto_run_rate:.1f}%",
    "",
    "Safety:",
    "- can_auto_run: false for all evaluated suggestions",
    "- execution requires explicit confirmation",
    "- no embeddings, semantic search, vector DB, or LLM scoring used",
    "",
]

if skipped_by_workflow:
    lines.append("Skipped future workflow prompts:")
    for workflow_id, count in sorted(skipped_by_workflow.items()):
        lines.append(f"- {workflow_id}: {count}")
    lines.append("")

if failures:
    lines.append("Failures:")
    for row, result, reason in failures:
        lines.append(
            f"- {row['id']} {row['workflow']} {row['level']}: {row['prompt']} -> {reason}; "
            f"status={result['status']} candidates={[item['workflow_id'] for item in result['candidates']]}"
        )
else:
    lines.append("Failures: none for evaluated prompts")

lines.extend([
    "",
    "Machine-readable results:",
    json.dumps({
        "approved_workflows": sorted(approved_ids),
        "dataset_total": counts["total"],
        "evaluated": evaluated,
        "skipped": counts["skipped"],
        "easy_accuracy": easy_accuracy,
        "medium_accuracy": medium_accuracy,
        "hard_clarification_rate": hard_rate,
        "false_auto_run_rate": false_auto_run_rate,
        "failed_count": len(failures),
        "skipped_by_workflow": dict(sorted(skipped_by_workflow.items())),
        "verdict": "ready" if not failures and easy_accuracy == 100.0 and medium_accuracy >= 90.0 and hard_rate >= 95.0 and false_auto_run_rate == 0.0 else "not_ready",
    }, indent=2),
])

report_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
print("\n".join(lines))

if failures or easy_accuracy < 100.0 or medium_accuracy < 90.0 or hard_rate < 95.0 or false_auto_run_rate != 0.0:
    raise SystemExit(1)
PY
