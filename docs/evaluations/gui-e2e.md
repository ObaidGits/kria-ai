# GUI E2E Evaluation Runbook

This runbook describes how to execute KRIA GUI automation tests in a safe, isolated environment.

## Required Services

1. Vision sidecar (`sidecars/kria-vision`)
2. Input/uinput daemon with required host permissions
3. KRIA runtime with GUI automation features enabled

## Setup

```bash
cd sidecars/kria-vision
python -m venv venv
source venv/bin/activate
pip install -r requirements.txt
python main.py
```

In another shell:

```bash
cargo build
sudo ./target/release/<uinput-daemon-binary>
```

## Execution

```bash
cargo test --package kria-core gui -- --nocapture
```

For full E2E harness execution, run the suite through `kria-eval` and record evidence artifacts.

## Safety Requirements

- Use a dedicated test session/workspace.
- Never run destructive GUI scenarios on a personal desktop session.
- Keep kill-switch and timeout policies enabled.

## Expected Evidence

- Input event traces
- GUI step execution logs
- Verification results with pass/fail reason
- Safety/policy decisions for gated actions
