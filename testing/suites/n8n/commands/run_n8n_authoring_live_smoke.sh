#!/usr/bin/env bash
# Live n8n authoring smoke. Creates and deletes only disposable KRIA Authoring Test workflows.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

BASE_URL="${N8N_BASE_URL:-${KRIA_N8N_BASE_URL:-http://127.0.0.1:5678}}"
API_KEY="${N8N_API_KEY:-${KRIA_N8N_API_KEY:-}}"
if [ -z "$API_KEY" ] && [ -f "$HOME/.kria/secrets/n8n_api_key" ]; then
    API_KEY="$(tr -d '\r\n' < "$HOME/.kria/secrets/n8n_api_key")"
fi

if [ -z "$API_KEY" ]; then
    echo "SKIP: n8n API key missing. Set N8N_API_KEY or save ~/.kria/secrets/n8n_api_key."
    exit 0
fi

python3 - "$BASE_URL" "$API_KEY" <<'PY'
import json
import sys
import time
import urllib.error
import urllib.request

base = sys.argv[1].rstrip("/")
api_key = sys.argv[2].strip()
prefix = "KRIA Authoring Test"
run_id = str(int(time.time() * 1000))
name = f"{prefix} {run_id}"
path = f"kria-authoring-live-smoke-{run_id}"
created_id = None


def request(method, url, payload=None, api=True, timeout=20):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    if api:
        headers["X-N8N-API-KEY"] = api_key
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read().decode("utf-8", errors="replace")
            if not body:
                return {}
            return json.loads(body)
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"{method} {url} failed HTTP {exc.code}: {body[:500]}") from exc


def workflow_payload():
    return {
        "name": name,
        "nodes": [
            {
                "id": "kria_authoring_webhook",
                "name": "Webhook",
                "type": "n8n-nodes-base.webhook",
                "typeVersion": 2.1,
                "position": [0, 0],
                "webhookId": f"kria-authoring-live-{run_id}",
                "parameters": {
                    "httpMethod": "POST",
                    "path": path,
                    "responseMode": "lastNode",
                    "options": {},
                },
            },
            {
                "id": "kria_authoring_http_lookup",
                "name": "HTTP Lookup",
                "type": "n8n-nodes-base.httpRequest",
                "typeVersion": 4.2,
                "position": [280, 0],
                "parameters": {
                    "method": "GET",
                    "url": "https://httpbin.org/get",
                    "sendQuery": True,
                    "queryParameters": {
                        "parameters": [
                            {
                                "name": "query",
                                "value": "={{ $json.body?.title || $json.query?.title || 'Inception' }}",
                            }
                        ]
                    },
                    "options": {},
                },
            },
            {
                "id": "kria_authoring_prepare_result",
                "name": "Prepare Result",
                "type": "n8n-nodes-base.set",
                "typeVersion": 3.4,
                "position": [560, 0],
                "parameters": {
                    "assignments": {
                        "assignments": [
                            {
                                "id": "kria_authoring_result",
                                "name": "result",
                                "type": "object",
                                "value": "={{ { source: 'HTTP Lookup', data: $json } }}",
                            }
                        ]
                    },
                    "options": {},
                },
            },
        ],
        "connections": {
            "Webhook": {
                "main": [[{"node": "HTTP Lookup", "type": "main", "index": 0}]]
            },
            "HTTP Lookup": {
                "main": [[{"node": "Prepare Result", "type": "main", "index": 0}]]
            },
        },
        "settings": {"executionOrder": "v1"},
    }


def list_workflows():
    payload = request("GET", f"{base}/api/v1/workflows")
    items = payload.get("data") or payload.get("workflows") or (payload if isinstance(payload, list) else [])
    return items


try:
    request("GET", f"{base}/api/v1/workflows?limit=1")
    created = request("POST", f"{base}/api/v1/workflows", workflow_payload())
    created_id = created.get("id")
    if not created_id:
        raise RuntimeError(f"create response did not include id: {created}")
    print(f"Created disposable workflow: {name} ({created_id})")

    detail = request("GET", f"{base}/api/v1/workflows/{created_id}")
    if detail.get("active"):
        raise RuntimeError("created workflow unexpectedly active")
    print("Verified draft inactive after create")

    request("POST", f"{base}/api/v1/workflows/{created_id}/activate")
    print("Activated disposable workflow for webhook smoke")

    webhook_url = f"{base}/webhook/{path}"
    result = request("POST", webhook_url, {"title": "Inception"}, api=False, timeout=45)
    rendered = json.dumps(result)[:800]
    if "Inception" not in rendered and "HTTP Lookup" not in rendered:
        raise RuntimeError(f"webhook result did not include expected output: {rendered}")
    print(f"Webhook smoke returned expected output preview: {rendered[:240]}")

finally:
    if created_id:
        try:
            request("DELETE", f"{base}/api/v1/workflows/{created_id}")
            print(f"Deleted disposable workflow: {created_id}")
        except Exception as exc:
            print(f"WARNING: cleanup delete failed for {created_id}: {exc}")
    try:
        leftovers = [
            item for item in list_workflows()
            if str(item.get("name", "")).startswith(prefix)
        ]
        print(json.dumps({"leftover_kria_authoring_test_workflows": leftovers, "count": len(leftovers)}))
        if leftovers:
            raise SystemExit(1)
    except Exception as exc:
        print(f"WARNING: leftover verification failed: {exc}")
        raise
PY
