#!/usr/bin/env python3
"""
F6 Study Analysis Script — Preregistered Analysis Only
=======================================================
Usage:
    .venv/bin/python scripts/memory_graph/study_analysis.py
    (Run from the KRIA repository root. Requires scipy and numpy from .venv.)

Reads:  evidence/F6/run-001/study/trial_data/aggregated.json
Writes: evidence/F6/run-001/study/analysis/results.json

Protocol: FROZEN per preregistration.json task_6_1_3 and task_6_1_4.

Statistical tests (the ONLY tests run):
  - Wilcoxon signed-rank, two-tailed, alpha=0.05 (scipy.stats.wilcoxon)
  - 95% bootstrap CI over paired median differences
    seed=0x4D475230, 10,000 samples (numpy Generator)
  - Median percentage difference: ((median_3D - median_2D) / median_2D) * 100

Exit codes:
  0 = analysis complete (results written; may be STUDY_GO or STUDY_NO_GO)
  1 = analysis error (bad input, missing fields, unexpected failure)
  2 = insufficient data (fewer than 20 valid pairs) — INCONCLUSIVE / NO-GO

POST-HOC THRESHOLD CHANGE PROHIBITION
======================================
The thresholds alpha=0.05, effect_size_threshold=-10.0, min_pairs=20,
bootstrap_samples=10000, bootstrap_seed=0x4D475230 are FROZEN at preregistration.
Any modification after data collection begins is a study protocol violation that
invalidates the F6 gate outcome.

Status: PENDING_EXECUTION — no study data collected yet (task 6.3.1 outstanding).
"""

from __future__ import annotations

import json
import sys
import os
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import numpy as np
from scipy import stats

# ---------------------------------------------------------------------------
# FROZEN preregistered constants — DO NOT CHANGE after data collection begins
# ---------------------------------------------------------------------------
ALPHA: float = 0.05
EFFECT_SIZE_THRESHOLD_PCT: float = -10.0   # ≤ -10% = ≥10% improvement
MIN_VALID_PAIRS: int = 20
BOOTSTRAP_SAMPLES: int = 10_000
BOOTSTRAP_SEED_HEX: str = "0x4D475230"
BOOTSTRAP_SEED_INT: int = 0x4D475230       # 1_296_519_728

SCHEMA_VERSION: str = "1.0.0"
PREREGISTRATION_REF: str = (
    "evidence/F6/run-001/study/preregistration.json"
)
SCRIPT_VERSION: str = "1.0.0"
PRIMARY_MEASURES: list[str] = ["task_completion_time_ms", "error_count"]
SECONDARY_MEASURES: list[str] = [
    "path_identification_accuracy",
    "time_to_first_correct_node",
]


# ---------------------------------------------------------------------------
# Utility helpers
# ---------------------------------------------------------------------------

def _utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _die(msg: str, code: int = 1) -> None:
    print(f"ERROR: {msg}", file=sys.stderr)
    sys.exit(code)


def _load_json(path: Path) -> Any:
    if not path.exists():
        _die(
            f"Input file not found: {path}\n"
            "Has the study been completed and aggregated.json produced? "
            "This script cannot run until task 6.3.1 is complete.",
            code=1,
        )
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def _require_field(obj: dict, field: str, context: str) -> Any:
    if field not in obj:
        _die(f"Required field '{field}' missing from {context}.")
    return obj[field]


# ---------------------------------------------------------------------------
# Data loading and pairing
# ---------------------------------------------------------------------------

def load_aggregated(path: Path) -> dict:
    data = _load_json(path)
    for field in ("condition_2D_list", "condition_3D", "pairing_key", "metadata"):
        _require_field(data, field, str(path))
    if data["pairing_key"] != "path_pair_id":
        _die("aggregated.json pairing_key must be 'path_pair_id' (frozen by preregistration).")
    return data


def build_valid_pairs(
    trials_2d: list[dict],
    trials_3d: list[dict],
) -> tuple[list[dict], list[dict], int]:
    """
    Match trials by path_pair_id (listwise deletion).
    Returns (paired_2d, paired_3d, n_dropped_listwise).
    """
    map_2d = {t["path_pair_id"]: t for t in trials_2d}
    map_3d = {t["path_pair_id"]: t for t in trials_3d}
    common = sorted(set(map_2d) & set(map_3d))
    all_ids = set(map_2d) | set(map_3d)
    n_dropped = len(all_ids) - len(common)
    paired_2d = [map_2d[pid] for pid in common]
    paired_3d = [map_3d[pid] for pid in common]
    return paired_2d, paired_3d, n_dropped


# ---------------------------------------------------------------------------
# Statistical analysis (preregistered only)
# ---------------------------------------------------------------------------

def bootstrap_ci_paired_median_diff(
    values_a: list[float],
    values_b: list[float],
    n_samples: int = BOOTSTRAP_SAMPLES,
    seed: int = BOOTSTRAP_SEED_INT,
    ci_level: float = 0.95,
) -> tuple[float, float]:
    """
    95% bootstrap CI over paired median differences.
    Resamples (a_i, b_i) pairs with replacement; records median(b - a)
    for each bootstrap replicate.
    Returns (ci_lower, ci_upper).
    """
    rng = np.random.default_rng(seed)
    diffs = np.array(values_b, dtype=float) - np.array(values_a, dtype=float)
    n = len(diffs)
    bootstrap_medians = np.empty(n_samples, dtype=float)
    for i in range(n_samples):
        sample = rng.choice(diffs, size=n, replace=True)
        bootstrap_medians[i] = np.median(sample)
    alpha = 1.0 - ci_level
    lo = float(np.percentile(bootstrap_medians, 100 * alpha / 2))
    hi = float(np.percentile(bootstrap_medians, 100 * (1.0 - alpha / 2)))
    return lo, hi


def analyze_measure(
    measure: str,
    paired_2d: list[dict],
    paired_3d: list[dict],
    all_valid_2d: list[dict],
    all_valid_3d: list[dict],
) -> dict:
    """
    Run the preregistered analysis for a single primary measure.
    Uses paired samples for Wilcoxon and bootstrap CI.
    Uses all valid (unpaired) samples for per-condition medians.
    """
    vals_2d_paired = [float(t[measure]) for t in paired_2d]
    vals_3d_paired = [float(t[measure]) for t in paired_3d]
    vals_2d_all = [float(t[measure]) for t in all_valid_2d if t.get(measure) is not None]
    vals_3d_all = [float(t[measure]) for t in all_valid_3d if t.get(measure) is not None]

    median_2d = float(np.median(vals_2d_all)) if vals_2d_all else float("nan")
    median_3d = float(np.median(vals_3d_all)) if vals_3d_all else float("nan")

    if median_2d == 0.0:
        pct_diff = float("nan")
    else:
        pct_diff = ((median_3d - median_2d) / median_2d) * 100.0

    # Wilcoxon signed-rank on paired values (two-tailed)
    try:
        w_stat, p_val = stats.wilcoxon(
            vals_3d_paired,
            vals_2d_paired,
            alternative="two-sided",
            zero_method="wilcox",
        )
        w_stat = float(w_stat)
        p_val = float(p_val)
    except ValueError as exc:
        _die(f"Wilcoxon test failed for {measure}: {exc}")

    significant = p_val < ALPHA

    ci_lo, ci_hi = bootstrap_ci_paired_median_diff(
        vals_2d_paired,
        vals_3d_paired,
    )

    # Direction: improvement (3D < 2D), degradation (3D > 2D), or no_change
    if pct_diff < 0:
        direction = "improvement"
    elif pct_diff > 0:
        direction = "degradation"
    else:
        direction = "no_change"

    return {
        "median_2D": median_2d,
        "median_3D": median_3d,
        "median_percentage_difference": pct_diff,
        "wilcoxon_W": w_stat,
        "wilcoxon_p_value": p_val,
        "wilcoxon_significant": significant,
        "ci_95_lower": ci_lo,
        "ci_95_upper": ci_hi,
        "ci_includes_zero": ci_lo <= 0.0 <= ci_hi,
        "direction": direction,
    }


# ---------------------------------------------------------------------------
# Secondary measures (exploratory only — no significance test)
# ---------------------------------------------------------------------------

def analyze_secondary(
    measure: str,
    valid_2d: list[dict],
    valid_3d: list[dict],
) -> dict:
    vals_2d = [float(t[measure]) for t in valid_2d if t.get(measure) is not None]
    vals_3d = [float(t[measure]) for t in valid_3d if t.get(measure) is not None]
    return {
        "median_2D": float(np.median(vals_2d)) if vals_2d else None,
        "median_3D": float(np.median(vals_3d)) if vals_3d else None,
        "n_valid_2D": len(vals_2d),
        "n_valid_3D": len(vals_3d),
        "note": (
            "Secondary exploratory measure. Not used in primary Wilcoxon "
            "analysis or GO/NO-GO determination per preregistration "
            "task_6_1_3.measures.secondary."
        ),
    }


# ---------------------------------------------------------------------------
# Learning / order effect time series
# ---------------------------------------------------------------------------

def build_time_series(
    trials: list[dict],
    condition_label: str,
) -> list[dict]:
    """
    Build chronologically ordered time series for learning/order effect
    inspection. Trials must already be sorted by (session_id, trial_id).
    """
    # Sort by session then trial within session
    sorted_trials = sorted(
        trials,
        key=lambda t: (t.get("session_id", 0), t.get("trial_id", 0)),
    )
    series = []
    for global_idx, t in enumerate(sorted_trials, start=1):
        series.append({
            "global_trial_index": global_idx,
            "session_id": t.get("session_id"),
            "session_label_abba": t.get("session_label_abba"),
            "path_pair_id": t["path_pair_id"],
            "task_completion_time_ms": t.get("task_completion_time_ms"),
            "error_count": t.get("error_count"),
        })
    return series


# ---------------------------------------------------------------------------
# GO / NO-GO determination (task_benefit_threshold TB-1 through TB-4)
# ---------------------------------------------------------------------------

def determine_go_no_go(
    valid_pairs_count: int,
    primary_results: dict[str, dict],
) -> dict:
    """
    Apply the four conjunctive TB conditions from preregistration task_6_1_4.

    TB-1: >= MIN_VALID_PAIRS (20) valid pairs
    TB-2: Wilcoxon p < 0.05 for at least one primary measure
    TB-3: median_percentage_difference <= -10.0 for the significant measure
    TB-4: No primary measure shows significant degradation (p < 0.05, positive direction)
    """
    # TB-1
    tb1 = valid_pairs_count >= MIN_VALID_PAIRS
    if not tb1:
        return {
            "verdict": "INCONCLUSIVE",
            "tb1_minimum_pairs_pass": False,
            "tb2_significance_pass": False,
            "tb3_effect_size_pass": False,
            "tb4_direction_pass": False,
            "explanation": (
                f"INCONCLUSIVE (treated as NO-GO): only {valid_pairs_count} valid pairs, "
                f"minimum required is {MIN_VALID_PAIRS}. Study is underpowered."
            ),
            "post_hoc_change_prohibition_acknowledged": True,
        }

    # TB-4: any significant degradation is an immediate NO-GO
    tb4 = True
    degradation_on = []
    for m, res in primary_results.items():
        if res["wilcoxon_significant"] and res["direction"] == "degradation":
            tb4 = False
            degradation_on.append(m)

    # TB-2
    significant_measures = [
        m for m, res in primary_results.items() if res["wilcoxon_significant"]
    ]
    tb2 = len(significant_measures) > 0

    # TB-3: effect size on the significant measure(s)
    tb3 = False
    if tb2:
        for m in significant_measures:
            pct = primary_results[m]["median_percentage_difference"]
            if not isinstance(pct, float) or not (pct != pct):  # not NaN
                if pct <= EFFECT_SIZE_THRESHOLD_PCT:
                    tb3 = True

    all_pass = tb1 and tb2 and tb3 and tb4
    verdict = "STUDY_GO" if all_pass else "STUDY_NO_GO"

    # Build explanation
    parts = []
    if not tb2:
        parts.append(
            f"TB-2 FAIL: Wilcoxon p >= {ALPHA} for all primary measures "
            f"(no statistically significant effect detected)."
        )
    if tb2 and not tb3:
        for m in significant_measures:
            pct = primary_results[m]["median_percentage_difference"]
            parts.append(
                f"TB-3 FAIL on {m}: median_percentage_difference = {pct:.2f}% "
                f"(threshold <= {EFFECT_SIZE_THRESHOLD_PCT}%; effect too small)."
            )
    if not tb4:
        parts.append(
            f"TB-4 FAIL: Statistically significant DEGRADATION detected on: "
            f"{', '.join(degradation_on)}. 3D is significantly worse, not better."
        )
    if all_pass:
        parts.append(
            "All TB-1 through TB-4 conditions pass: study component is GO. "
            "Full F6 GO still requires fps_threshold, idle_quiet_threshold, "
            "resource_parity_threshold, a11y_parity_threshold, "
            "core_task_parity_threshold, and foss_closure_threshold from "
            "separate evidence artifacts (preregistration task_6_1_4)."
        )

    return {
        "verdict": verdict,
        "tb1_minimum_pairs_pass": tb1,
        "tb2_significance_pass": tb2,
        "tb3_effect_size_pass": tb3,
        "tb4_direction_pass": tb4,
        "explanation": " ".join(parts) if parts else "All conditions passed.",
        "post_hoc_change_prohibition_acknowledged": True,
    }


# ---------------------------------------------------------------------------
# Exclusion summary builder
# ---------------------------------------------------------------------------

def build_exclusion_summary(
    data: dict,
    n_dropped_listwise: int,
) -> dict:
    meta = data["metadata"]
    total_2d = meta.get("total_trials_2D", 0)
    excl_2d = meta.get("excluded_trials_2D", 0)
    total_3d = meta.get("total_trials_3D", 0)
    excl_3d = meta.get("excluded_trials_3D", 0)
    excl_sessions = meta.get("excluded_sessions", 0)

    rate_2d = (excl_2d / total_2d * 100.0) if total_2d > 0 else 0.0
    rate_3d = (excl_3d / total_3d * 100.0) if total_3d > 0 else 0.0

    # Harvest per-reason details from metadata if available
    exclusion_details = meta.get("exclusion_details", [])

    return {
        "total_trials_2D": total_2d,
        "excluded_trials_2D": excl_2d,
        "total_trials_3D": total_3d,
        "excluded_trials_3D": excl_3d,
        "excluded_sessions": excl_sessions,
        "pairs_dropped_listwise": n_dropped_listwise,
        "exclusion_rate_2D_pct": round(rate_2d, 2),
        "exclusion_rate_3D_pct": round(rate_3d, 2),
        "exclusion_details": exclusion_details,
        "high_exclusion_rate_warning": (rate_2d > 20.0 or rate_3d > 20.0),
    }


# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------

def main() -> None:
    # Resolve paths relative to repo root (script lives in scripts/memory_graph/)
    repo_root = Path(__file__).resolve().parent.parent.parent
    evidence_base = repo_root / ".kiro" / "specs" / \
        "memory-graph-production-redesign" / "evidence" / "F6" / "run-001" / "study"
    input_path = evidence_base / "trial_data" / "aggregated.json"
    output_dir = evidence_base / "analysis"
    output_path = output_dir / "results.json"

    print(f"F6 Study Analysis — reading: {input_path}")

    data = load_aggregated(input_path)
    trials_2d: list[dict] = data["condition_2D_list"]
    trials_3d: list[dict] = data["condition_3D"]

    # Validate required fields in trial records
    for cond_name, trials in [("condition_2D_list", trials_2d), ("condition_3D", trials_3d)]:
        for t in trials:
            for f in ("path_pair_id", "task_completion_time_ms", "error_count"):
                _require_field(t, f, f"{cond_name} trial")

    # Build valid pairs via listwise deletion
    paired_2d, paired_3d, n_dropped = build_valid_pairs(trials_2d, trials_3d)
    valid_pairs_count = len(paired_2d)

    print(f"  Valid 2D trials: {len(trials_2d)}, 3D trials: {len(trials_3d)}")
    print(f"  Valid paired trials: {valid_pairs_count} (dropped listwise: {n_dropped})")

    # Enforce minimum valid pairs — exit code 2 for INCONCLUSIVE
    if valid_pairs_count < MIN_VALID_PAIRS:
        print(
            f"\nINSUFFICIENT DATA: {valid_pairs_count} valid pairs < "
            f"{MIN_VALID_PAIRS} required minimum.\n"
            "Result: INCONCLUSIVE — treated as NO-GO per preregistration task_6_1_4 TB-1.\n"
            "The study must be re-run with sufficient trials before analysis can proceed.",
            file=sys.stderr,
        )
        sys.exit(2)

    # Primary measures analysis
    primary_results: dict[str, dict] = {}
    for measure in PRIMARY_MEASURES:
        print(f"  Analyzing primary measure: {measure}")
        primary_results[measure] = analyze_measure(
            measure, paired_2d, paired_3d, trials_2d, trials_3d
        )

    # Secondary measures (exploratory)
    secondary_results: dict[str, dict] = {}
    for measure in SECONDARY_MEASURES:
        secondary_results[measure] = analyze_secondary(measure, trials_2d, trials_3d)

    # Learning / order effect time series
    ts_2d = build_time_series(trials_2d, "2D_list")
    ts_3d = build_time_series(trials_3d, "3D")

    # Exclusion summary
    exclusion_summary = build_exclusion_summary(data, n_dropped)
    if exclusion_summary["high_exclusion_rate_warning"]:
        print(
            "  WARNING: Exclusion rate > 20% in at least one condition. "
            "This must be discussed in the analysis report.",
            file=sys.stderr,
        )

    # GO/NO-GO determination
    go_no_go = determine_go_no_go(valid_pairs_count, primary_results)

    # Assemble output
    results = {
        "schema_version": SCHEMA_VERSION,
        "analysis_timestamp_utc": _utc_now(),
        "preregistration_reference": PREREGISTRATION_REF,
        "analysis_script_version": SCRIPT_VERSION,
        "bootstrap_seed_hex": BOOTSTRAP_SEED_HEX,
        "bootstrap_samples": BOOTSTRAP_SAMPLES,
        "valid_pairs_count": valid_pairs_count,
        "exclusion_summary": exclusion_summary,
        "primary_measures": primary_results,
        "learning_order_effect": {
            "condition_2D_list_time_series": ts_2d,
            "condition_3D_time_series": ts_3d,
        },
        "secondary_measures": secondary_results,
        "go_no_go_determination": go_no_go,
    }

    output_dir.mkdir(parents=True, exist_ok=True)
    with open(output_path, "w", encoding="utf-8") as fh:
        json.dump(results, fh, indent=2)
        fh.write("\n")

    print(f"\nAnalysis complete. Results written to: {output_path}")
    verdict = go_no_go["verdict"]
    print(f"Study outcome: {verdict}")
    print(f"Explanation: {go_no_go['explanation']}")


if __name__ == "__main__":
    main()
