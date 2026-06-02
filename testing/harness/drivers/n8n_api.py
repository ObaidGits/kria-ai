from __future__ import annotations

import json
import os
import hashlib
import time
import tomllib
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from testing.harness.reporting.redaction import redact_json

DISPOSABLE_PREFIXES = (
    "KRIA Desktop Command E2E",
    "KRIA Desktop Live E2E",
    "KRIA E2E Test",
    "KRIA Test",
    "KRIA Authoring Test",
    "KRIA CRUD Test",
)


def _api_key() -> str:
    for name in ("N8N_API_KEY", "KRIA_N8N_API_KEY"):
        value = os.environ.get(name, "").strip()
        if value:
            return value
    for path in (
        Path.home() / ".kria" / "secrets" / "n8n_api_key",
        Path.home() / ".kria" / "secrets" / "n8n_api.key",
        Path.home() / ".kria" / "secrets" / "n8n-api-key",
    ):
        if path.exists():
            value = path.read_text(encoding="utf-8").strip()
            if value:
                return value
    return ""


def resolve_n8n_base_url(root_dir: Path | None = None, explicit: str | None = None) -> str:
    if explicit and explicit.strip():
        return explicit.strip()
    for name in ("N8N_BASE_URL", "KRIA_N8N_BASE_URL"):
        value = os.environ.get(name, "").strip()
        if value:
            return value
    if root_dir is not None:
        config_path = root_dir / "config" / "default.toml"
        if config_path.exists():
            try:
                data = tomllib.loads(config_path.read_text(encoding="utf-8"))
                value = str(data.get("n8n", {}).get("base_url") or "").strip()
                if value:
                    return value
            except (OSError, tomllib.TOMLDecodeError):
                pass
    return "http://127.0.0.1:5678"


def request_n8n(
    *,
    base_url: str,
    method: str,
    path: str,
    body: dict[str, Any] | None = None,
    timeout_seconds: int = 30,
) -> dict[str, Any]:
    key = _api_key()
    if not key:
        return {"ok": False, "error": "missing n8n API key"}
    data = json.dumps(body).encode("utf-8") if body is not None else None
    headers = {"X-N8N-API-KEY": key, "Content-Type": "application/json"}
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}{path}", data=data, headers=headers, method=method
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            raw = response.read().decode("utf-8", errors="replace")
            try:
                parsed = json.loads(raw) if raw else {}
            except json.JSONDecodeError:
                parsed = {"raw": raw}
            return {"ok": 200 <= response.status < 300, "status": response.status, "data": redact_json(parsed)}
    except urllib.error.HTTPError as error:
        raw = error.read().decode("utf-8", errors="replace")
        return {"ok": False, "status": error.code, "error": raw}
    except OSError as error:
        return {"ok": False, "error": str(error)}


def n8n_health(base_url: str, timeout_seconds: int = 5) -> dict[str, Any]:
    for path in ("/healthz", "/"):
        try:
            request = urllib.request.Request(f"{base_url.rstrip('/')}{path}", method="GET")
            with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
                if response.status < 500:
                    return {"ok": True, "status": response.status, "path": path}
        except OSError as error:
            last_error = str(error)
    return {"ok": False, "error": last_error if "last_error" in locals() else "unreachable"}


def list_workflows(
    *, base_url: str, limit: int = 100, timeout_seconds: int = 30
) -> dict[str, Any]:
    return request_n8n(
        base_url=base_url,
        method="GET",
        path=f"/api/v1/workflows?limit={limit}",
        timeout_seconds=timeout_seconds,
    )


def get_workflow(
    *, base_url: str, workflow_id: str, timeout_seconds: int = 30
) -> dict[str, Any]:
    return request_n8n(
        base_url=base_url,
        method="GET",
        path=f"/api/v1/workflows/{workflow_id}",
        timeout_seconds=timeout_seconds,
    )


def create_disposable_workflow(
    *, base_url: str, name: str, timeout_seconds: int = 30
) -> dict[str, Any]:
    if not is_disposable_workflow_name(name):
        return {"ok": False, "error": f"workflow name '{name}' is not disposable"}
    path = f"kria-desktop-command-{int(time.time() * 1000)}"
    payload = {
        "name": name,
        "nodes": [
            {
                "id": "kria_desktop_command_webhook",
                "name": "Webhook",
                "type": "n8n-nodes-base.webhook",
                "typeVersion": 2,
                "position": [0, 0],
                "webhookId": path,
                "parameters": {
                    "httpMethod": "POST",
                    "path": path,
                    "responseMode": "responseNode",
                    "options": {},
                },
            },
            {
                "id": "kria_desktop_command_response",
                "name": "Respond to KRIA",
                "type": "n8n-nodes-base.respondToWebhook",
                "typeVersion": 1.1,
                "position": [260, 0],
                "parameters": {
                    "respondWith": "json",
                    "responseBody": '={{ JSON.stringify({ ok: true, source: "KRIA Desktop Command E2E" }) }}',
                    "options": {},
                },
            },
        ],
        "connections": {
            "Webhook": {
                "main": [[{"node": "Respond to KRIA", "type": "main", "index": 0}]]
            }
        },
        "settings": {"executionOrder": "v1"},
    }
    created = request_n8n(
        base_url=base_url,
        method="POST",
        path="/api/v1/workflows",
        body=payload,
        timeout_seconds=timeout_seconds,
    )
    workflow = created.get("data") if isinstance(created.get("data"), dict) else {}
    return {
        "ok": bool(created.get("ok")) and bool(workflow.get("id")),
        "status": created.get("status"),
        "workflow": workflow_summary(workflow),
        "workflow_id": workflow.get("id"),
        "name": workflow.get("name") or name,
        "detail": _minimal_api_result(created),
    }


def workflow_rows(payload: dict[str, Any]) -> list[dict[str, Any]]:
    data = payload.get("data")
    if isinstance(data, dict):
        rows = data.get("data", data.get("workflows", []))
    else:
        rows = data
    if isinstance(rows, list):
        return [row for row in rows if isinstance(row, dict)]
    return []


def is_disposable_workflow_name(name: str) -> bool:
    return name.startswith(DISPOSABLE_PREFIXES)


def delete_workflow_if_disposable(
    *, base_url: str, workflow_id: str, allow_workflow_ids: list[str] | None = None
) -> dict[str, Any]:
    detail = get_workflow(base_url=base_url, workflow_id=workflow_id)
    if not detail.get("ok"):
        return {
            "ok": False,
            "status": "not_deleted",
            "reason": "failed to fetch workflow before delete",
            "detail": redact_json(detail),
        }
    workflow = detail.get("data")
    if not isinstance(workflow, dict):
        return {
            "ok": False,
            "status": "not_deleted",
            "reason": "workflow detail was not an object",
        }
    name = str(workflow.get("name") or "")
    allowed_by_run_capture = workflow_id in set(allow_workflow_ids or [])
    if not is_disposable_workflow_name(name) and not allowed_by_run_capture:
        return {
            "ok": False,
            "status": "not_deleted",
            "reason": f"workflow name '{name}' is not disposable",
        }
    deleted = request_n8n(
        base_url=base_url,
        method="DELETE",
        path=f"/api/v1/workflows/{workflow_id}",
    )
    return {
        "ok": bool(deleted.get("ok")),
        "status": "deleted" if deleted.get("ok") else "not_deleted",
        "workflow": workflow_summary(workflow),
        "detail": _minimal_api_result(deleted),
    }


def find_workflows_by_prefix(*, base_url: str, prefix: str) -> dict[str, Any]:
    payload = list_workflows(base_url=base_url, limit=250)
    if not payload.get("ok"):
        return payload
    matches = [
        {"id": row.get("id"), "name": row.get("name")}
        for row in workflow_rows(payload)
        if str(row.get("name") or "").startswith(prefix)
    ]
    return {"ok": True, "matches": redact_json(matches)}


def delete_disposable_workflows_by_prefix(*, base_url: str, prefix: str) -> dict[str, Any]:
    if not is_disposable_workflow_name(prefix):
        return {"ok": False, "reason": f"prefix '{prefix}' is not disposable"}
    matches = find_workflows_by_prefix(base_url=base_url, prefix=prefix)
    if not matches.get("ok"):
        return {"ok": False, "reason": "failed to list workflows", "detail": _minimal_api_result(matches)}
    actions = []
    failed = False
    for row in matches.get("matches") or []:
        workflow_id = str(row.get("id") or "")
        if not workflow_id:
            continue
        result = delete_workflow_if_disposable(base_url=base_url, workflow_id=workflow_id)
        actions.append({"workflow_id": workflow_id, "name": row.get("name"), "ok": result.get("ok")})
        failed = failed or not result.get("ok")
    return {"ok": not failed, "deleted": actions, "count": len(actions)}


def workflow_summary(workflow: Any) -> dict[str, Any]:
    if not isinstance(workflow, dict):
        return {}
    nodes = workflow.get("nodes")
    connections = workflow.get("connections")
    return {
        "id": workflow.get("id"),
        "name": workflow.get("name"),
        "active": workflow.get("active"),
        "node_count": len(nodes) if isinstance(nodes, list) else 0,
        "connection_count": len(connections) if isinstance(connections, dict) else 0,
        "hash": workflow_semantic_hash(workflow),
    }


def workflow_semantic_hash(workflow: Any) -> str:
    if not isinstance(workflow, dict):
        return ""
    keep = {
        "name": workflow.get("name"),
        "nodes": workflow.get("nodes", []),
        "connections": workflow.get("connections", {}),
        "settings": workflow.get("settings", {}),
        "active": workflow.get("active"),
    }
    encoded = json.dumps(redact_json(keep), sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def workflow_hash(*, base_url: str, workflow_id: str) -> dict[str, Any]:
    detail = get_workflow(base_url=base_url, workflow_id=workflow_id)
    if not detail.get("ok"):
        return {"ok": False, "detail": _minimal_api_result(detail)}
    workflow = detail.get("data")
    return {"ok": True, "hash": workflow_semantic_hash(workflow), "workflow": workflow_summary(workflow)}


def update_workflow_name_if_disposable(
    *,
    base_url: str,
    workflow_id: str,
    suffix: str,
    timeout_seconds: int = 30,
    allow_workflow_ids: list[str] | None = None,
) -> dict[str, Any]:
    detail = get_workflow(base_url=base_url, workflow_id=workflow_id, timeout_seconds=timeout_seconds)
    if not detail.get("ok"):
        return {"ok": False, "reason": "failed to fetch workflow before update", "detail": _minimal_api_result(detail)}
    workflow = detail.get("data")
    if not isinstance(workflow, dict):
        return {"ok": False, "reason": "workflow detail was not an object"}
    name = str(workflow.get("name") or "")
    allowed_by_run_capture = workflow_id in set(allow_workflow_ids or [])
    if not is_disposable_workflow_name(name) and not allowed_by_run_capture:
        return {"ok": False, "reason": f"workflow name '{name}' is not disposable"}
    next_name = name if name.endswith(suffix) else f"{name}{suffix}"
    payload = {
        "name": next_name,
        "nodes": workflow.get("nodes", []),
        "connections": workflow.get("connections", {}),
        "settings": workflow.get("settings", {"executionOrder": "v1"}),
    }
    updated = request_n8n(
        base_url=base_url,
        method="PUT",
        path=f"/api/v1/workflows/{workflow_id}",
        body=payload,
        timeout_seconds=timeout_seconds,
    )
    return {
        "ok": bool(updated.get("ok")),
        "before": workflow_summary(workflow),
        "after": workflow_summary(updated.get("data")),
        "detail": _minimal_api_result(updated),
    }


def wait_for_workflow(
    *,
    base_url: str,
    workflow_id: str,
    should_exist: bool = True,
    timeout_seconds: int = 20,
    interval_seconds: float = 0.5,
) -> dict[str, Any]:
    deadline = time.time() + timeout_seconds
    last: dict[str, Any] = {}
    while time.time() <= deadline:
        detail = get_workflow(base_url=base_url, workflow_id=workflow_id, timeout_seconds=5)
        exists = bool(detail.get("ok"))
        last = detail
        if exists == should_exist:
            return {"ok": True, "exists": exists, "workflow": workflow_summary(detail.get("data"))}
        time.sleep(interval_seconds)
    return {"ok": False, "exists": bool(last.get("ok")), "detail": _minimal_api_result(last)}


def run_n8n_action(
    *,
    action: dict[str, Any],
    variables: dict[str, Any],
    root_dir: Path,
) -> tuple[bool, str, dict[str, Any], dict[str, Any]]:
    base_url = resolve_n8n_base_url(root_dir, action.get("base_url"))
    kind = action.get("kind")
    outputs: dict[str, Any] = {}
    if kind == "create_disposable_workflow":
        name = _substitute(str(action.get("name") or f"{variables.get('run_prefix', '')} Workflow"), variables)
        if not name:
            return False, "create_disposable_workflow missing name", {}, outputs
        result = create_disposable_workflow(base_url=base_url, name=name)
        output_var = str(action.get("output_var") or "n8n_workflow_id")
        name_var = str(action.get("name_output_var") or "n8n_workflow_name")
        if result.get("ok"):
            outputs[output_var] = str(result.get("workflow_id") or "")
            outputs[name_var] = str(result.get("name") or name)
            created = variables.setdefault("_created_n8n_workflow_ids", [])
            if isinstance(created, list) and outputs[output_var]:
                created.append(outputs[output_var])
        return bool(result.get("ok")), f"created disposable workflow {name}", {"result": result}, outputs
    if kind == "store_workflow_hash":
        workflow_id = _value_from_verification(action, variables, {})
        if not workflow_id:
            return False, "store_workflow_hash missing workflow id", {}, outputs
        result = workflow_hash(base_url=base_url, workflow_id=str(workflow_id))
        output_var = action.get("output_var") or action.get("hash_var")
        if result.get("ok") and output_var:
            outputs[str(output_var)] = result.get("hash")
        return bool(result.get("ok")), f"stored workflow hash for {workflow_id}", {"result": result}, outputs
    if kind == "mutate_workflow_name":
        workflow_id = _value_from_verification(action, variables, {})
        if not workflow_id:
            return False, "mutate_workflow_name missing workflow id", {}, outputs
        suffix = str(action.get("suffix") or f" Drifted {variables.get('run_id', '')}")
        suffix = _substitute(suffix, variables)
        result = update_workflow_name_if_disposable(
            base_url=base_url,
            workflow_id=str(workflow_id),
            suffix=suffix,
            allow_workflow_ids=_allowed_created_workflow_ids(variables, action),
        )
        if result.get("ok") and action.get("hash_var"):
            outputs[str(action["hash_var"])] = (result.get("after") or {}).get("hash")
        return bool(result.get("ok")), f"mutated disposable workflow name for {workflow_id}", {"result": result}, outputs
    if kind == "delete_disposable_workflow":
        workflow_id = _value_from_verification(action, variables, {})
        if not workflow_id:
            return False, "delete_disposable_workflow missing workflow id", {}, outputs
        result = delete_workflow_if_disposable(
            base_url=base_url,
            workflow_id=str(workflow_id),
            allow_workflow_ids=_allowed_created_workflow_ids(variables, action),
        )
        return bool(result.get("ok")), f"deleted disposable workflow {workflow_id}", {"result": result}, outputs
    if kind == "delete_disposable_workflows_by_prefix":
        prefix = _substitute(str(action.get("prefix") or variables.get("run_prefix") or ""), variables)
        if not prefix:
            return False, "delete_disposable_workflows_by_prefix missing prefix", {}, outputs
        result = delete_disposable_workflows_by_prefix(base_url=base_url, prefix=prefix)
        return bool(result.get("ok")), f"deleted disposable workflows by prefix {prefix}", {"result": result}, outputs
    if kind == "wait_for_workflow":
        workflow_id = _value_from_verification(action, variables, {})
        if not workflow_id:
            return False, "wait_for_workflow missing workflow id", {}, outputs
        result = wait_for_workflow(
            base_url=base_url,
            workflow_id=str(workflow_id),
            should_exist=bool(action.get("should_exist", True)),
            timeout_seconds=int(action.get("timeout_seconds") or 20),
        )
        return bool(result.get("ok")), f"waited for workflow {workflow_id}", {"result": result}, outputs
    return False, f"unknown n8n action kind: {kind}", {}, outputs


def run_n8n_verification(
    *,
    verification: dict[str, Any],
    variables: dict[str, Any],
    response: dict[str, Any],
    root_dir: Path,
) -> tuple[bool, str, dict[str, Any]]:
    from testing.harness.assertions.chat_response import first_path_value

    base_url = resolve_n8n_base_url(root_dir, verification.get("base_url"))
    kind = verification.get("kind")
    if kind == "workflow_exists":
        workflow_id = _value_from_verification(verification, variables, response)
        if not workflow_id:
            return False, "workflow_exists verification missing workflow id", {}
        detail = get_workflow(base_url=base_url, workflow_id=str(workflow_id))
        return bool(detail.get("ok")), f"workflow {workflow_id} exists", {"workflow": workflow_summary(detail.get("data")), "detail": _minimal_api_result(detail)}
    if kind == "workflow_inactive":
        workflow_id = _value_from_verification(verification, variables, response)
        if not workflow_id:
            return False, "workflow_inactive verification missing workflow id", {}
        detail = get_workflow(base_url=base_url, workflow_id=str(workflow_id))
        if not detail.get("ok"):
            return False, f"workflow {workflow_id} could not be fetched", {"detail": detail}
        data = detail.get("data")
        active = data.get("active") if isinstance(data, dict) else None
        return active in (False, None), f"workflow {workflow_id} inactive", {"workflow": workflow_summary(data), "active": active}
    if kind == "workflow_active":
        workflow_id = _value_from_verification(verification, variables, response)
        if not workflow_id:
            return False, "workflow_active verification missing workflow id", {}
        detail = get_workflow(base_url=base_url, workflow_id=str(workflow_id))
        if not detail.get("ok"):
            return False, f"workflow {workflow_id} could not be fetched", {"detail": _minimal_api_result(detail)}
        data = detail.get("data")
        active = data.get("active") if isinstance(data, dict) else None
        return active is True, f"workflow {workflow_id} active", {"workflow": workflow_summary(data), "active": active}
    if kind == "workflow_missing":
        workflow_id = _value_from_verification(verification, variables, response)
        if not workflow_id:
            return False, "workflow_missing verification missing workflow id", {}
        detail = get_workflow(base_url=base_url, workflow_id=str(workflow_id))
        return not bool(detail.get("ok")), f"workflow {workflow_id} missing", {"detail": _minimal_api_result(detail)}
    if kind == "workflow_hash_unchanged":
        workflow_id = _value_from_verification(verification, variables, response)
        expected = variables.get(str(verification.get("hash_var") or "original_hash"))
        if not workflow_id or not expected:
            return False, "workflow_hash_unchanged missing workflow id or expected hash", {}
        result = workflow_hash(base_url=base_url, workflow_id=str(workflow_id))
        actual = result.get("hash")
        return bool(result.get("ok")) and actual == expected, f"workflow {workflow_id} hash unchanged", {"expected": expected, "actual": actual, "workflow": result.get("workflow")}
    if kind == "workflow_name_has_prefix":
        workflow_id = _value_from_verification(verification, variables, response)
        prefix = _substitute(str(verification.get("prefix") or variables.get("run_prefix") or ""), variables)
        if not workflow_id or not prefix:
            return False, "workflow_name_has_prefix missing workflow id or prefix", {}
        detail = get_workflow(base_url=base_url, workflow_id=str(workflow_id))
        workflow = detail.get("data")
        name = str(workflow.get("name") or "") if isinstance(workflow, dict) else ""
        return bool(detail.get("ok")) and name.startswith(prefix), f"workflow {workflow_id} name has prefix {prefix}", {"workflow": workflow_summary(workflow), "prefix": prefix}
    if kind == "workflow_count_by_prefix":
        prefix = _substitute(str(verification.get("prefix") or variables.get("run_prefix") or ""), variables)
        expected = verification.get("equals")
        matches = find_workflows_by_prefix(base_url=base_url, prefix=prefix)
        if not matches.get("ok"):
            return False, "failed to list workflows for prefix count", {"detail": _minimal_api_result(matches)}
        count = len(matches.get("matches") or [])
        ok = count == expected if isinstance(expected, int) else count >= int(verification.get("min", 0))
        return ok, f"workflow count by prefix {prefix} is {count}", {"count": count, "matches": matches.get("matches")}
    if kind == "workflow_exists_by_name_prefix":
        prefix = _substitute(str(verification.get("prefix") or variables.get("run_prefix") or ""), variables)
        matches = find_workflows_by_prefix(base_url=base_url, prefix=prefix)
        if not matches.get("ok"):
            return False, "failed to list workflows for prefix existence", {"detail": _minimal_api_result(matches)}
        found = matches.get("matches") or []
        return len(found) > 0, f"workflow exists by prefix {prefix}", {"matches": found[:10]}
    if kind == "no_workflows_with_prefix":
        prefix = str(verification.get("prefix") or variables.get("run_prefix") or "")
        prefix = _substitute(prefix, variables)
        if not prefix:
            return False, "no_workflows_with_prefix missing prefix", {}
        matches = find_workflows_by_prefix(base_url=base_url, prefix=prefix)
        if not matches.get("ok"):
            return False, "failed to list workflows for leftover check", {"detail": matches}
        found = matches.get("matches") or []
        return len(found) == 0, f"no workflows with prefix {prefix}", {"matches": found}
    if kind == "workflow_still_exists":
        workflow_id = _value_from_verification(verification, variables, response)
        if not workflow_id:
            return False, "workflow_still_exists verification missing workflow id", {}
        detail = get_workflow(base_url=base_url, workflow_id=str(workflow_id))
        return bool(detail.get("ok")), f"workflow {workflow_id} still exists", {"workflow": workflow_summary(detail.get("data")), "detail": _minimal_api_result(detail)}
    if kind == "response_workflow_exists":
        workflow_id = first_path_value(
            response,
            [
                "n8n.result.n8n_workflow_id",
                "n8n.result.workflow.n8n_workflow_id",
                "n8n.result.workflow.n8nWorkflowId",
            ],
        )
        if not workflow_id:
            return False, "response did not contain n8n workflow id", {}
        detail = get_workflow(base_url=base_url, workflow_id=str(workflow_id))
        return bool(detail.get("ok")), f"response workflow {workflow_id} exists", {"workflow": workflow_summary(detail.get("data")), "detail": _minimal_api_result(detail)}
    if kind == "no_workflows_with_run_prefix":
        prefix = str(variables.get("run_prefix") or "")
        if not prefix:
            return False, "run prefix was not available", {}
        matches = find_workflows_by_prefix(base_url=base_url, prefix=prefix)
        if not matches.get("ok"):
            return False, "failed to list workflows for run-prefix leftover check", {"detail": _minimal_api_result(matches)}
        found = matches.get("matches") or []
        return len(found) == 0, f"no workflows with run prefix {prefix}", {"matches": found}
    return False, f"unknown n8n verification kind: {kind}", {}


def _value_from_verification(
    verification: dict[str, Any], variables: dict[str, Any], response: dict[str, Any]
) -> Any:
    from testing.harness.assertions.json_path import get_path

    if "value" in verification:
        return verification["value"]
    if "var" in verification:
        return variables.get(str(verification["var"]))
    if "response_path" in verification:
        return get_path(response, str(verification["response_path"]))
    return None


def _minimal_api_result(result: Any) -> dict[str, Any]:
    if not isinstance(result, dict):
        return {}
    minimal = {
        "ok": result.get("ok"),
        "status": result.get("status"),
        "error": result.get("error"),
    }
    data = result.get("data")
    if isinstance(data, dict):
        minimal["workflow"] = workflow_summary(data)
    return redact_json({key: value for key, value in minimal.items() if value not in (None, "")})


def _substitute(value: str, variables: dict[str, Any]) -> str:
    result = value
    for key, item in variables.items():
        result = result.replace("${" + key + "}", str(item))
    return result


def _allowed_created_workflow_ids(
    variables: dict[str, Any], action: dict[str, Any] | None = None
) -> list[str]:
    if action and not action.get("allow_created_in_run", True):
        return []
    created = variables.get("_created_n8n_workflow_ids")
    if not isinstance(created, list):
        return []
    return [str(item) for item in created if item not in (None, "")]
