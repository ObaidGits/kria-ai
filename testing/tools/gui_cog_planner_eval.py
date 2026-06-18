#!/usr/bin/env python3
"""Task 9 OFFLINE planner accuracy gate.

Runs the LlmPlanner decomposition against the LIVE model server using the SAME
system prompt + JSON schema the Rust planner uses (loaded from the crate via
`include_str!`-shared files), and scores EXACT sub-goal KIND-sequence accuracy
against the labeled fixtures. Bar: >= 85% (Requirement 23.1).

This is the gate that must pass BEFORE the planner is wired into the live loop
(Requirement 17.2/17.4). Until it passes, the deterministic fallback stays the
active path.

Usage:
  python3 testing/tools/gui_cog_planner_eval.py [--endpoint URL] [--model ID]

Exit code 0 iff accuracy >= bar.
"""
import argparse
import json
import os
import re
import sys
import time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PROMPT_FILE = os.path.join(ROOT, "crates/kria-core/src/agent/gui_cognition_v2/planner_prompt.txt")
SCHEMA_FILE = os.path.join(ROOT, "crates/kria-core/src/agent/gui_cognition_v2/planner_schema.json")
FIXTURES_FILE = os.path.join(ROOT, "testing/tools/gui_cog_planner_fixtures.json")

KNOWN_KINDS = {
    "open_app", "click", "type", "navigate", "run_command",
    "write_file", "read_output", "verify", "other",
}


def discover_endpoint() -> str:
    """Find the live llama-server OpenAI endpoint (env override or ss scan)."""
    env = os.environ.get("KRIA_PLANNER_ENDPOINT")
    if env:
        return env.rstrip("/")
    # Scan listening llama-server ports.
    try:
        import subprocess
        out = subprocess.run(["ss", "-ltnp"], capture_output=True, text=True, timeout=5).stdout
        for line in out.splitlines():
            if "llama-server" in line:
                m = re.search(r"127\.0\.0\.1:(\d+)", line)
                if m:
                    port = m.group(1)
                    url = f"http://127.0.0.1:{port}/v1"
                    try:
                        code = urllib.request.urlopen(f"{url}/models", timeout=3).status
                        if code == 200:
                            return url
                    except Exception:
                        continue
    except Exception:
        pass
    return "http://127.0.0.1:8080/v1"


def model_id(endpoint: str, override: str | None) -> str:
    if override:
        return override
    try:
        body = json.loads(urllib.request.urlopen(f"{endpoint}/models", timeout=5).read())
        data = body.get("data") or []
        if data:
            return data[0].get("id", "")
    except Exception:
        pass
    return ""


def extract_json_object(content: str) -> str | None:
    depth = 0
    start = None
    for i, ch in enumerate(content):
        if ch == "{":
            if depth == 0:
                start = i
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0 and start is not None:
                return content[start:i + 1]
    return None


def decompose(endpoint: str, mid: str, schema: dict, prompt: str, task: str, timeout: float):
    payload = {
        "model": mid,
        "messages": [
            {"role": "system", "content": prompt},
            {"role": "user", "content": f"Task: {task.strip()}"},
        ],
        "temperature": 0.1,
        "max_tokens": 768,
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": "plan", "schema": schema, "strict": True},
        },
    }
    req = urllib.request.Request(
        f"{endpoint}/chat/completions",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        body = json.loads(r.read())
    content = body["choices"][0]["message"]["content"]
    obj = extract_json_object(content)
    if not obj:
        raise ValueError(f"no JSON object in: {content[:120]!r}")
    data = json.loads(obj)
    kinds = []
    for sg in data.get("sub_goals", []):
        k = str(sg.get("kind", "")).strip().lower()
        kinds.append(k if k in KNOWN_KINDS else "other")
    if not kinds:
        raise ValueError("empty sub_goals")
    return kinds


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--endpoint", default=None)
    ap.add_argument("--model", default=None)
    ap.add_argument("--bar", type=float, default=0.85)
    ap.add_argument("--timeout", type=float, default=60.0)
    args = ap.parse_args()

    prompt = open(PROMPT_FILE).read()
    schema = json.load(open(SCHEMA_FILE))
    fixtures = json.load(open(FIXTURES_FILE))["fixtures"]

    endpoint = (args.endpoint or discover_endpoint()).rstrip("/")
    mid = model_id(endpoint, args.model)
    print(f"endpoint={endpoint} model={mid!r} fixtures={len(fixtures)} bar={args.bar:.0%}\n")

    results = []
    correct = 0
    for fx in fixtures:
        accepts = fx.get("accept") or [fx["expect"]]
        try:
            got = decompose(endpoint, mid, schema, prompt, fx["prompt"], args.timeout)
            ok = any(got == a for a in accepts)
        except Exception as e:
            got = f"ERROR: {e}"
            ok = False
        correct += 1 if ok else 0
        mark = "OK " if ok else "XX "
        print(f"  {mark}{fx['id']}: {fx['prompt'][:48]!r}\n        expect={fx['expect']} got={got}")
        results.append({"id": fx["id"], "prompt": fx["prompt"], "expect": fx["expect"],
                        "accept": accepts, "got": got, "ok": ok})

    n = len(fixtures)
    acc = correct / n if n else 0.0
    print(f"\n=== PLANNER OFFLINE GATE ===")
    print(f"accuracy = {correct}/{n} = {acc:.1%}  (bar {args.bar:.0%}) -> {'PASS' if acc >= args.bar else 'FAIL'}")

    out_dir = os.path.join(ROOT, "eval_reports", "gui_cog")
    os.makedirs(out_dir, exist_ok=True)
    out_path = os.path.join(out_dir, f"planner_gate_{time.strftime('%Y%m%d_%H%M%S')}.json")
    json.dump({"endpoint": endpoint, "model": mid, "accuracy": acc, "correct": correct,
               "total": n, "bar": args.bar, "pass": acc >= args.bar, "results": results},
              open(out_path, "w"), indent=2)
    print(f"artifact: {out_path}")
    return 0 if acc >= args.bar else 1


if __name__ == "__main__":
    sys.exit(main())
