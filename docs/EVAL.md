# KRIA Evaluation Harness

> **Last Updated:** 2026-05-11

---

## Overview

KRIA includes an evaluation harness (`kria-eval` crate) for measuring agent quality across test cases. It uses LLM-based judging to score responses.

---

## Components

| Component | Purpose |
|-----------|---------|
| `runner` | Executes eval cases |
| `judge` | LLM-based evaluation |
| `report` | Generates reports |
| `sandbox` | Docker test environment |

---

## Test Case Schema

```yaml
id: eval-001
name: "File search accuracy"
prompt: "Find all PDF files in ~/Documents"
expected:
  tool_calls:
    - search_files
    - list_directory
  outcome: "Lists PDF files"
scoring:
  correctness: 1.0
  tool_selection: 0.5
  efficiency: 0.3
```

---

## Running Evals

```bash
# Run all evals
cargo run -p kria-eval

# Run specific eval
cargo run -p kria-eval -- --filter eval-001

# Generate report
cargo run -p kria-eval -- --report output.json
```

---

## Scoring

| Metric | Weight | Description |
|--------|--------|-------------|
| Correctness | 40% | Did it achieve the goal? |
| Tool Selection | 30% | Were the right tools used? |
| Efficiency | 20% | Minimal unnecessary steps? |
| Safety | 10% | No policy violations? |

---

## Source Files

- `crates/kria-eval/src/runner.rs`
- `crates/kria-eval/src/judge.rs`
- `crates/kria-eval/src/sandbox.rs`
